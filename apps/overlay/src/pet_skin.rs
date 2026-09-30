//! 桌宠皮肤库：扫描 `ui/assets/skins/<id>/skin.json` 皮肤包，
//! 提供清单解析、路径校验与列表查询。
//!
//! 皮肤包格式：
//! ```json
//! {
//!   "id": "lingxi-hamster",
//!   "name": "灵犀仓鼠",
//!   "author": "LingXi",
//!   "version": "1.0.0",
//!   "description": "…",
//!   "states": {
//!     "idle":      { "image": "idle.png",     "bubble": "灵犀" },
//!     "thinking":  { "image": "thinking.png", "bubble": "思考中…" },
//!     "speaking":  { "image": "speaking.png", "bubble": "建议好了" },
//!     "alert":     { "image": "alert.png",    "bubble": "QQ 新消息" }
//!   }
//! }
//! ```
//! 校验规则：id 只允许 `[A-Za-z0-9_-]`（防路径穿越）；四个状态图片必须存在；
//! 图片文件名不得包含路径分隔符。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_SKIN_ID: &str = "petdex-ai-hoshino";
pub const PET_STATES: [&str; 4] = ["idle", "thinking", "speaking", "alert"];

/// 皮肤来源标识：内置（安装包内，不可删）。
pub const SKIN_SOURCE_BUILTIN: &str = "builtin";
/// 皮肤来源标识：用户目录（市场下载落位点，可删）。
pub const SKIN_SOURCE_USER: &str = "user";

/// 用户皮肤目录：`%APPDATA%/lingxi/skins`（市场下载落位点，可删除）。
pub fn user_skins_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lingxi").join("skins"))
}

/// 单帧尺寸声明（spritesheet 皮肤必填）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetSkinFrame {
    pub width: u32,
    pub height: u32,
}

/// 一个状态的两种形态：
/// - 静态图（旧格式）：`image: "idle.png"`
/// - 帧动画（petdex spritesheet）：`sheet` + `row` + `frames` + `durationMs`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetSkinState {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub sheet: Option<String>,
    #[serde(default)]
    pub row: Option<u32>,
    #[serde(default)]
    pub frames: Option<u32>,
    #[serde(default)]
    pub duration_ms: Option<u32>,
    #[serde(default)]
    pub bubble: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetSkinManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// spritesheet 皮肤的单帧尺寸；静态图皮肤可省略。
    #[serde(default)]
    pub frame: Option<PetSkinFrame>,
    pub states: BTreeMap<String, PetSkinState>,
}

/// 发给前端的帧动画描述（一个状态用 spritesheet 的某一行循环播放）。
#[derive(Debug, Clone, Serialize)]
pub struct PetSheetAnim {
    /// 相对 `ui/` 的 spritesheet URL。
    pub sheet: String,
    pub row: u32,
    pub frames: u32,
    pub duration_ms: u32,
    /// 网格列数 / 行数（由图片实际尺寸与声明帧尺寸推得）。
    pub cols: u32,
    pub rows: u32,
}

/// 读取图片像素尺寸：支持 PNG 与 WebP（VP8X / VP8L / VP8 有损）。
/// 解析失败返回 None（调用方按 8×9 网格兜底）。
fn sheet_dims(path: &std::path::Path) -> Option<(u32, u32)> {
    let data = std::fs::read(path).ok()?;
    if data.len() >= 24 && data.starts_with(&[0x89, b'P', b'N', b'G']) {
        let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        if w > 0 && h > 0 {
            return Some((w, h));
        }
        return None;
    }
    if data.len() > 30 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        match &data[12..16] {
            b"VP8X" => {
                let w = (data[24] as u32 | (data[25] as u32) << 8 | (data[26] as u32) << 16) + 1;
                let h = (data[27] as u32 | (data[28] as u32) << 8 | (data[29] as u32) << 16) + 1;
                return Some((w, h));
            }
            b"VP8L" => {
                let v = data[21] as u32
                    | (data[22] as u32) << 8
                    | (data[23] as u32) << 16
                    | (data[24] as u32) << 24;
                let w = (v & 0x3FFF) + 1;
                let h = ((v >> 14) & 0x3FFF) + 1;
                return Some((w, h));
            }
            b"VP8 "
                if data.len() >= 30 && data[23] == 0x9D && data[24] == 0x01 && data[25] == 0x2A =>
            {
                // 有损帧：3 字节帧标签后跟起始码 9D 01 2A，再是 14 位宽高。
                let w = ((data[26] as u32 | (data[27] as u32) << 8) & 0x3FFF) + 1;
                let h = ((data[28] as u32 | (data[29] as u32) << 8) & 0x3FFF) + 1;
                return Some((w, h));
            }
            _ => {}
        }
    }
    None
}

/// 由图片实际尺寸与声明帧尺寸推算网格；解析失败按 petdex 经典 8×9 兜底。
fn sheet_grid(sheet_path: &std::path::Path, frame: &PetSkinFrame) -> (u32, u32) {
    match sheet_dims(sheet_path) {
        Some((w, h)) => (
            (w / frame.width.max(1)).max(1),
            (h / frame.height.max(1)).max(1),
        ),
        None => (8, 9),
    }
}

/// 设置页皮肤列表条目。
#[derive(Debug, Clone, Serialize)]
pub struct PetSkinInfo {
    pub id: String,
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
    /// 相对 `ui/` 的缩略图 URL（idle 态图片）。
    pub thumbnail: String,
    /// 皮肤来源：builtin（内置）/ user（用户目录）。
    pub source: String,
}

/// 桌宠整体配置（发给桌宠窗口与设置页的完整视图）。
#[derive(Debug, Clone, Serialize)]
pub struct PetSkinView {
    pub skin: PetSkinManifest,
    /// 相对 `ui/` 的状态图 URL，键为状态名（仅静态图状态）。
    pub images: BTreeMap<String, String>,
    /// 帧动画状态描述（仅 spritesheet 状态）。
    pub anims: BTreeMap<String, PetSheetAnim>,
    /// 最终生效的气泡文案（用户覆盖优先，缺省回落皮肤默认），键为状态名。
    pub bubbles: BTreeMap<String, String>,
    /// 用户原始覆盖值（设置页回填输入框用；None = 未覆盖）。
    pub overrides: PetBubbleOverrides,
    pub visible: bool,
}

/// 用户气泡文案覆盖（空 = 使用皮肤默认文案）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PetBubbleOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert: Option<String>,
}

impl PetBubbleOverrides {
    pub fn text_for(&self, state: &str) -> Option<&str> {
        match state {
            "idle" => self.idle.as_deref(),
            "thinking" => self.thinking.as_deref(),
            "speaking" => self.speaking.as_deref(),
            "alert" => self.alert.as_deref(),
            _ => None,
        }
    }
}

/// 皮肤 id 合法性：`[A-Za-z0-9_-]+`，防路径穿越。
pub fn valid_skin_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 图片文件名合法性：非空且不含路径分隔符 / 上跳。
fn valid_image_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && !name.contains(':')
}

/// 定位皮肤目录：从当前可执行文件向上找 `ui/assets/skins`。
/// 不依赖编译期路径，仓库整体移动后无需重编即可生效。
pub fn skins_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .skip(1)
        .map(|ancestor| ancestor.join("ui").join("assets").join("skins"))
        .find(|candidate| candidate.join(DEFAULT_SKIN_ID).join("skin.json").is_file())
}

/// 解析皮肤来源：用户目录优先于内置目录；都不存在返回 None。
pub fn skin_source(id: &str) -> Option<&'static str> {
    if !valid_skin_id(id) {
        return None;
    }
    if let Some(user_root) = user_skins_dir() {
        if user_root.join(id).join("skin.json").is_file() {
            return Some(SKIN_SOURCE_USER);
        }
    }
    let builtin_root = skins_dir()?;
    if builtin_root.join(id).join("skin.json").is_file() {
        return Some(SKIN_SOURCE_BUILTIN);
    }
    None
}

/// 皮肤目录（id 已校验）：用户目录优先，其次内置目录。
pub fn skin_dir(id: &str) -> Option<PathBuf> {
    if !valid_skin_id(id) {
        return None;
    }
    if skin_source(id) == Some(SKIN_SOURCE_USER) {
        return user_skins_dir().map(|root| root.join(id));
    }
    skins_dir().map(|root| root.join(id))
}

/// 皮肤内文件 → 前端可直显 URL：
/// 内置皮肤返回相对 `ui/` 的路径（webview 根即 ui/）；
/// 用户皮肤返回 asset protocol URL（需 tauri.conf.json 开启 assetProtocol）。
pub fn skin_file_url(source: &str, skin_id: &str, file_name: &str) -> String {
    if source == SKIN_SOURCE_USER {
        if let Some(path) = user_skins_dir().map(|root| root.join(skin_id).join(file_name)) {
            return asset_url(&path);
        }
    }
    format!("assets/skins/{skin_id}/{file_name}")
}

/// Windows asset protocol URL：`http://asset.localhost/<encodeURIComponent(path)>`。
/// 逐字节百分号编码 UTF-8，保留字符与前端 `convertFileSrc` 完全一致
/// （字母数字与 `-_.~!*'()` 保留，其余—including `/` 与 `\`—编码为 %XX）。
fn asset_url(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut out = String::from("http://asset.localhost/");
    for &byte in raw.as_bytes() {
        let keep = matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
                | b'-' | b'_' | b'.' | b'~' | b'!' | b'*' | b'\'' | b'(' | b')'
        );
        if keep {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// 解析并校验皮肤清单。目录必须存在且每个状态四选一：静态图或完整
/// spritesheet 动画配置；两者都缺 / 都有视为非法。
pub fn load_manifest(id: &str) -> Result<PetSkinManifest, String> {
    let dir = skin_dir(id).ok_or_else(|| format!("皮肤目录不可用：{id}"))?;
    load_manifest_at(&dir, id)
}

/// 解析并校验指定目录中的皮肤清单（市场安装前预校验，目录无需已注册）。
pub fn load_manifest_at(dir: &Path, id: &str) -> Result<PetSkinManifest, String> {
    let manifest_path = dir.join("skin.json");
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("读取皮肤清单失败（{}）：{error}", manifest_path.display()))?;
    let manifest: PetSkinManifest =
        serde_json::from_str(&raw).map_err(|error| format!("皮肤清单格式错误（{id}）：{error}"))?;
    if manifest.id != id {
        return Err(format!(
            "皮肤目录与清单 id 不一致：目录 {id}，清单 {}",
            manifest.id
        ));
    }
    if manifest.name.trim().is_empty() {
        return Err(format!("皮肤 {id} 缺少 name"));
    }
    for state in PET_STATES {
        let entry = manifest
            .states
            .get(state)
            .ok_or_else(|| format!("皮肤 {id} 缺少 {state} 状态"))?;
        let has_image = entry.image.as_deref().is_some_and(|name| !name.is_empty());
        let sheet = entry.sheet.as_deref().filter(|name| !name.is_empty());
        if has_image == sheet.is_some() {
            return Err(format!(
                "皮肤 {id} 的 {state} 必须且只能指定 image 或 sheet 之一"
            ));
        }
        if let Some(name) = sheet {
            if !valid_image_name(name) {
                return Err(format!("皮肤 {id} 的 {state} sheet 名非法：{name}"));
            }
            if !dir.join(name).is_file() {
                return Err(format!("皮肤 {id} 的 {state} spritesheet 缺失：{name}"));
            }
            let frame = manifest
                .frame
                .as_ref()
                .ok_or_else(|| format!("皮肤 {id} 使用 spritesheet 但缺少 frame 帧尺寸声明"))?;
            if frame.width == 0 || frame.height == 0 {
                return Err(format!("皮肤 {id} 的 frame 尺寸必须为正数"));
            }
            if entry.row.is_none() || entry.frames.is_none() {
                return Err(format!("皮肤 {id} 的 {state} 缺少 row 或 frames"));
            }
        } else if let Some(name) = entry.image.as_deref() {
            if !valid_image_name(name) {
                return Err(format!("皮肤 {id} 的 {state} 图片名非法：{name}"));
            }
            if !dir.join(name).is_file() {
                return Err(format!("皮肤 {id} 的 {state} 图片缺失：{name}"));
            }
        }
    }
    Ok(manifest)
}

/// 列出全部有效皮肤（内置 + 用户，用户源优先；无效目录跳过并记日志）。
pub fn list_skins() -> Vec<PetSkinInfo> {
    let mut skins = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let roots = [
        user_skins_dir().map(|root| (root, SKIN_SOURCE_USER)),
        skins_dir().map(|root| (root, SKIN_SOURCE_BUILTIN)),
    ];
    for (root, source) in roots.into_iter().flatten() {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(id) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !valid_skin_id(&id) || !entry.path().is_dir() || seen.contains(&id) {
                continue;
            }
            let manifest = match load_manifest(&id) {
                Ok(manifest) => manifest,
                Err(error) => {
                    eprintln!("[lingxi] pet_skin: 跳过无效皮肤 {id}：{error}");
                    continue;
                }
            };
            seen.push(id.clone());
            // 缩略图：静态图取 idle 图片；spritesheet 取 idle 行第一帧
            //（`<url>#<row>`，前端按帧尺寸裁剪显示）。
            let thumbnail = manifest
                .states
                .get("idle")
                .map(|state| {
                    if let Some(image) = state.image.as_deref().filter(|n| !n.is_empty()) {
                        skin_file_url(source, &id, image)
                    } else {
                        let row = state.row.unwrap_or(0);
                        let sheet = state.sheet.clone().unwrap_or_default();
                        format!("{}#{row}", skin_file_url(source, &id, &sheet))
                    }
                })
                .unwrap_or_default();
            skins.push(PetSkinInfo {
                id: manifest.id,
                name: manifest.name,
                author: manifest.author,
                version: manifest.version,
                description: manifest.description,
                thumbnail,
                source: source.to_string(),
            });
        }
    }
    skins.sort_by(|a, b| a.id.cmp(&b.id));
    skins
}

/// 构建发给前端的完整视图。指定的皮肤无效时回落默认皮肤。
pub fn view_for(
    skin_id: &str,
    overrides: &PetBubbleOverrides,
    visible: bool,
) -> Result<PetSkinView, String> {
    let manifest = load_manifest(skin_id)
        .or_else(|error| {
            eprintln!("[lingxi] pet_skin: 皮肤 {skin_id} 不可用（{error}），回落默认皮肤");
            load_manifest(DEFAULT_SKIN_ID)
        })
        .map_err(|error| format!("默认皮肤也不可用：{error}"))?;
    let source = skin_source(&manifest.id).unwrap_or(SKIN_SOURCE_BUILTIN);
    let mut images = BTreeMap::new();
    let mut anims = BTreeMap::new();
    // 网格按 sheet 文件名缓存：同一张图多个状态只解析一次尺寸。
    let mut grids: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    let frame_fallback = PetSkinFrame {
        width: 192,
        height: 208,
    };
    for (state, entry) in manifest.states.iter() {
        if let Some(name) = entry.image.as_deref().filter(|n| !n.is_empty()) {
            images.insert(state.clone(), skin_file_url(source, &manifest.id, name));
        } else if let Some(sheet) = entry.sheet.as_deref().filter(|n| !n.is_empty()) {
            let (cols, rows) = *grids.entry(sheet.to_string()).or_insert_with(|| {
                let frame = manifest.frame.as_ref().unwrap_or(&frame_fallback);
                skin_dir(&manifest.id)
                    .map(|dir| sheet_grid(&dir.join(sheet), frame))
                    .unwrap_or((8, 9))
            });
            // 行号越界保护；frames 不超过一行容纳的列数，避免取帧跨行错位。
            let row = entry.row.unwrap_or(0).min(rows.saturating_sub(1));
            let frames = entry.frames.unwrap_or(1).max(1).min(cols.max(1));
            anims.insert(
                state.clone(),
                PetSheetAnim {
                    sheet: skin_file_url(source, &manifest.id, sheet),
                    row,
                    frames,
                    duration_ms: entry.duration_ms.unwrap_or(900).max(80),
                    cols,
                    rows,
                },
            );
        }
    }
    let bubbles = manifest
        .states
        .iter()
        .map(|(state, entry)| {
            let text = overrides
                .text_for(state)
                .map(str::to_string)
                .unwrap_or_else(|| entry.bubble.clone());
            (state.clone(), text)
        })
        .collect();
    Ok(PetSkinView {
        skin: manifest,
        images,
        anims,
        bubbles,
        overrides: overrides.clone(),
        visible,
    })
}
