//! 插件市场：清单获取/合并、皮肤包下载安装与卸载。
//!
//! V1 市场商品 = 皮肤包。清单双源：内置 `ui/assets/market/market.json`
//! （离线兜底、可演示）+ 设置项 `market_url`（远程清单，按 id 远程优先）。
//! 安装目标为用户皮肤目录（见 pet_skin::user_skins_dir），内置皮肤不可覆盖/删除。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{Emitter, State};

use crate::pet_skin;
use crate::state::{AppState, MutexExt};

/// zip 下载与解压限额：最多文件数。
const MAX_FILES: usize = 200;
/// zip 下载与解压限额：累计解压字节数（50 MB）。
const MAX_UNCOMPRESSED: u64 = 50 * 1024 * 1024;
/// 解压时拒绝的扩展名（小写、无点）。
/// 原生可执行内容：任何商品都禁止（不可审查、绕过签名体系）。
const NATIVE_BLOCKED_EXTS: [&str; 6] = ["exe", "dll", "vbs", "scr", "msi", "com"];
/// 脚本类型：皮肤禁止；工具插件放行（脚本是工具插件的交付物）。
const SCRIPT_BLOCKED_EXTS: [&str; 3] = ["bat", "cmd", "ps1"];

/// 市场清单条目（market.json 的 `entries` 元素）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: Option<u64>,
    /// 卡片预览图直链（可选）。缺省或非法时前端用色块占位。
    #[serde(default)]
    pub thumbnail: String,
    /// 商品类型："skin"（皮肤包）| "tool"（工具插件）；缺省 skin。
    #[serde(default = "default_kind")]
    pub kind: String,
}

fn default_kind() -> String {
    "skin".into()
}

/// 市场清单文件结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketCatalog {
    #[serde(default)]
    pub entries: Vec<MarketEntry>,
}

/// 发给前端的条目：清单字段 + 本机安装状态。
#[derive(Debug, Clone, Serialize)]
pub struct MarketItem {
    pub id: String,
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
    /// 卡片预览图直链；空串 = 前端用色块占位。
    pub thumbnail: String,
    pub size: Option<u64>,
    /// 已安装来源："builtin" | "user"；未安装为空串。
    pub installed_source: String,
    /// 已安装版本；未安装为空串。
    pub installed_version: String,
    /// true = 本机已有更低版本，按钮应显示「更新」。
    pub updatable: bool,
    /// "skin" | "tool"。
    pub kind: String,
}

/// 内置清单定位：内置皮肤根（ui/assets/skins）的上级目录下 market/market.json。
pub fn builtin_catalog_path() -> Option<PathBuf> {
    pet_skin::skins_dir().and_then(|skins| {
        skins
            .parent()
            .map(|assets| assets.join("market").join("market.json"))
    })
}

/// 插件根目录：%APPDATA%/lingxi/plugins（工具插件下载落位点）。
pub fn plugins_root() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lingxi").join("plugins"))
}

/// 全量同步插件工具到注册表：按 plugin_tool_map 移除旧项 → 重扫目录注册 →
/// 重建映射。启动与市场安装/卸载工具后各调一次；目录不存在时等价于清空。
/// 锁序恒为 tool_registry → plugin_tool_map，与他处一致。
pub(crate) fn sync_plugin_tools(state: &AppState) {
    let root = plugins_root().unwrap_or_else(|| PathBuf::from("."));
    let mut registry = state.tool_registry.safe_lock();
    for name in state.plugin_tool_map.safe_lock().values() {
        registry.remove(name);
    }
    let mut map = std::collections::HashMap::new();
    for plugin in lingxi_tools::plugin::scan_plugins(&root) {
        map.insert(plugin.manifest().id.clone(), plugin.manifest().name.clone());
        registry.register(plugin);
    }
    *state.plugin_tool_map.safe_lock() = map;
}

/// 版本 → 可比较序列：按 . - + 切分，每段解析 u64，失败记 0。
pub fn version_rank(version: &str) -> Vec<u64> {
    version
        .split(['.', '-', '+'])
        .map(|part| part.trim().parse::<u64>().unwrap_or(0))
        .collect()
}

/// candidate 是否严格比 current 新（逐位比较，缺位补 0，相等视为不新）。
pub fn version_newer(candidate: &str, current: &str) -> bool {
    let (a, b) = (version_rank(candidate), version_rank(current));
    let len = a.len().max(b.len());
    for i in 0..len {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// sha256 字段合法性：64 位十六进制。
pub fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// 下载地址合法性：仅允许 http/https。
pub fn valid_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// 单条清单校验：id/name/url/sha256 任一非法即丢弃该条目。
pub fn sanitize_entry(entry: MarketEntry) -> Option<MarketEntry> {
    if !pet_skin::valid_skin_id(&entry.id) {
        eprintln!("[lingxi] market: 丢弃非法 id 条目：{}", entry.id);
        return None;
    }
    if entry.name.trim().is_empty() {
        eprintln!("[lingxi] market: 丢弃缺少 name 的条目：{}", entry.id);
        return None;
    }
    if !valid_url(&entry.url) {
        eprintln!("[lingxi] market: 丢弃非法 url 的条目：{}", entry.id);
        return None;
    }
    if !valid_sha256(&entry.sha256) {
        eprintln!("[lingxi] market: 丢弃非法 sha256 的条目：{}", entry.id);
        return None;
    }
    if entry.kind != "skin" && entry.kind != "tool" {
        eprintln!(
            "[lingxi] market: 丢弃未知 kind 的条目：{}（{}）",
            entry.id, entry.kind
        );
        return None;
    }
    // 缩略图非法只清空、不丢条目：预览是加分项，不影响安装能力。
    let mut entry = entry;
    if !entry.thumbnail.is_empty() && !valid_url(&entry.thumbnail) {
        entry.thumbnail.clear();
    }
    Some(entry)
}

/// 拉取远程清单文本：30 秒超时、最多 3 次重定向、清单体积上限 1 MB。
fn fetch_catalog_text(url: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .redirects(3)
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|error| format!("拉取市场清单失败：{error}"))?;
    let mut text = String::new();
    response
        .into_reader()
        .take(1024 * 1024)
        .read_to_string(&mut text)
        .map_err(|error| format!("读取市场清单失败：{error}"))?;
    Ok(text)
}

/// 解析清单文本：JSON 结构错误返回 Err；单条字段非法则丢弃该条（记日志）。
pub fn parse_catalog(text: &str) -> Result<MarketCatalog, String> {
    let catalog: MarketCatalog =
        serde_json::from_str(text).map_err(|error| format!("市场清单格式错误：{error}"))?;
    let entries = catalog
        .entries
        .into_iter()
        .filter_map(sanitize_entry)
        .collect();
    Ok(MarketCatalog { entries })
}

/// 合并清单：内置为兜底；设置了 `market_url` 且拉取成功时，远程条目按 id 优先覆盖。
/// 远程任何环节失败 → 静默降级为仅内置清单（记日志，不向用户报错）。
pub fn merged_entries(state: &AppState) -> Vec<MarketEntry> {
    let mut entries: Vec<MarketEntry> = Vec::new();
    if let Some(path) = builtin_catalog_path() {
        match std::fs::read_to_string(&path) {
            Ok(text) => match parse_catalog(&text) {
                Ok(catalog) => entries.extend(catalog.entries),
                Err(error) => eprintln!("[lingxi] market: 内置清单解析失败：{error}"),
            },
            Err(error) => eprintln!("[lingxi] market: 读取内置清单失败：{error}"),
        }
    }
    let remote_url = {
        let settings = state.backend.safe_lock();
        settings.market_url.trim().to_string()
    };
    if remote_url.is_empty() {
        return entries;
    }
    match fetch_catalog_text(&remote_url).and_then(|text| parse_catalog(&text)) {
        Ok(remote) => {
            for entry in remote.entries {
                match entries.iter_mut().find(|existing| existing.id == entry.id) {
                    Some(existing) => *existing = entry,
                    None => entries.push(entry),
                }
            }
        }
        Err(error) => eprintln!("[lingxi] market: 远程清单不可用，降级内置清单：{error}"),
    }
    entries
}

/// 查询皮肤本机安装状态：Some((来源, 版本))。清单校验失败视为未安装。
fn installed_info(id: &str) -> Option<(String, String)> {
    let source = pet_skin::skin_source(id)?;
    let version = pet_skin::load_manifest(id).ok()?.version;
    Some((source.to_string(), version))
}

/// 工具插件安装状态：目录存在且清单合法即视为已安装（来源固定 user）。
fn installed_tool_info(id: &str) -> Option<(String, String)> {
    let dir = plugins_root()?.join(id);
    let plugin = lingxi_tools::plugin::PluginTool::load(&dir).ok()?;
    Some((
        pet_skin::SKIN_SOURCE_USER.to_string(),
        plugin.manifest().version.clone(),
    ))
}

/// 市场页数据：合并清单 + 本机安装状态标注。
/// #[tauri::command(async)]：远程清单拉取最长 30s，同步命令默认跑在主线程，会冻结整个窗口。
#[tauri::command(async)]
pub(crate) fn market_list(state: State<AppState>) -> Result<Vec<MarketItem>, String> {
    let entries = merged_entries(state.inner());
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        let (installed_source, installed_version) = match entry.kind.as_str() {
            "tool" => installed_tool_info(&entry.id).unwrap_or_default(),
            _ => installed_info(&entry.id).unwrap_or_default(),
        };
        let updatable =
            !installed_version.is_empty() && version_newer(&entry.version, &installed_version);
        items.push(MarketItem {
            id: entry.id,
            name: entry.name,
            author: entry.author,
            version: entry.version,
            description: entry.description,
            size: entry.size,
            thumbnail: entry.thumbnail,
            installed_source,
            installed_version,
            updatable,
            kind: entry.kind,
        });
    }
    Ok(items)
}

/// 当前毫秒时间戳（临时目录名去重用）。
fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

/// 下载 url 到 dest_path，边下边算 sha256；返回十六进制摘要。
/// 体积与 MAX_UNCOMPRESSED 共用限额，超限即中止。
fn download_to(url: &str, dest_path: &Path) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .redirects(3)
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|error| format!("下载失败：{error}"))?;
    let mut reader = response.into_reader();
    let file =
        std::fs::File::create(dest_path).map_err(|error| format!("创建下载文件失败：{error}"))?;
    let mut writer = std::io::BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("下载中断：{error}"))?;
        if read == 0 {
            break;
        }
        downloaded += read as u64;
        if downloaded > MAX_UNCOMPRESSED {
            return Err("下载内容超过大小限额".into());
        }
        hasher.update(&buffer[..read]);
        writer
            .write_all(&buffer[..read])
            .map_err(|error| format!("写入下载文件失败：{error}"))?;
    }
    writer
        .flush()
        .map_err(|error| format!("写入下载文件失败：{error}"))?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// 安全解压 zip 到目标目录：`enclosed_name` 防 zip-slip、扩展名黑名单、
/// 文件数与累计体积限额。zip 路径穿越 / 夹带可执行文件时直接报错。
/// 工具插件（allow_scripts）放行脚本类型，但原生可执行内容一律禁止。
fn safe_extract(zip_path: &Path, dest_dir: &Path, allow_scripts: bool) -> Result<(), String> {
    let file =
        std::fs::File::open(zip_path).map_err(|error| format!("打开下载文件失败：{error}"))?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|error| format!("压缩包读取失败：{error}"))?;
    if archive.len() > MAX_FILES {
        return Err("压缩包文件数超过限额".into());
    }
    let mut total: u64 = 0;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("压缩包读取失败：{error}"))?;
        let Some(relative) = entry.enclosed_name() else {
            return Err("压缩包内含非法路径（疑似路径穿越）".into());
        };
        if let Some(ext) = relative
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
        {
            let native = NATIVE_BLOCKED_EXTS.contains(&ext.as_str());
            let script = SCRIPT_BLOCKED_EXTS.contains(&ext.as_str());
            if native || (script && !allow_scripts) {
                return Err(format!("压缩包内含禁止的文件类型：{ext}"));
            }
        }
        let out_path = dest_dir.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|error| format!("创建目录失败：{error}"))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| format!("创建目录失败：{error}"))?;
        }
        total += entry.size();
        if total > MAX_UNCOMPRESSED {
            return Err("解压内容超过大小限额".into());
        }
        let mut out_file = std::fs::File::create(&out_path)
            .map_err(|error| format!("写入解压文件失败：{error}"))?;
        std::io::copy(&mut entry, &mut out_file)
            .map_err(|error| format!("写入解压文件失败：{error}"))?;
    }
    Ok(())
}

/// 兼容 zip 带顶层目录的情况：根部没有清单但唯一子目录里有，
/// 则把该子目录内容上移到根部（包标准结构 = 根部即插件目录）。
fn flatten_single_root(dir: &Path, manifest_file: &str) -> Result<(), String> {
    if dir.join(manifest_file).is_file() {
        return Ok(());
    }
    let entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|error| format!("读取临时目录失败：{error}"))?
        .collect::<Result<_, _>>()
        .map_err(|error| format!("读取临时目录失败：{error}"))?;
    if entries.len() != 1 || !entries[0].path().is_dir() {
        return Err(format!("压缩包结构不符合要求（根部缺少 {manifest_file}）"));
    }
    let inner = entries[0].path();
    for entry in std::fs::read_dir(&inner)
        .map_err(|error| format!("读取压缩包子目录失败：{error}"))?
        .flatten()
    {
        let target = dir.join(entry.file_name());
        std::fs::rename(entry.path(), &target)
            .map_err(|error| format!("整理压缩包结构失败：{error}"))?;
    }
    std::fs::remove_dir(&inner).map_err(|error| format!("整理压缩包结构失败：{error}"))?;
    Ok(())
}

/// 下载并安装市场商品（皮肤/工具插件）到用户目录（含升级覆盖与失败回滚）。
/// #[tauri::command(async)]：下载耗时秒级，同步命令会阻塞主线程冻结窗口。
#[tauri::command(async)]
pub(crate) fn market_install(
    app: tauri::AppHandle,
    state: State<AppState>,
    id: String,
) -> Result<(), String> {
    // 皮肤与插件 id 同字符集规则，复用校验。
    if !pet_skin::valid_skin_id(&id) {
        return Err("id 非法".into());
    }
    let entry = merged_entries(state.inner())
        .into_iter()
        .find(|candidate| candidate.id == id)
        .ok_or_else(|| format!("市场清单中找不到条目：{id}"))?;
    let is_tool = entry.kind == "tool";
    if !is_tool && pet_skin::skin_source(&id) == Some(pet_skin::SKIN_SOURCE_BUILTIN) {
        return Err("与内置皮肤冲突，无法安装".into());
    }
    let user_root = if is_tool {
        plugins_root().ok_or_else(|| "无法定位插件目录".to_string())?
    } else {
        pet_skin::user_skins_dir().ok_or_else(|| "无法定位用户皮肤目录".to_string())?
    };
    std::fs::create_dir_all(&user_root).map_err(|error| format!("创建用户目录失败：{error}"))?;

    let tmp_dir = user_root.join(format!(".tmp-{id}-{}", now_millis()));
    if tmp_dir.exists() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
    std::fs::create_dir_all(&tmp_dir).map_err(|error| format!("创建临时目录失败：{error}"))?;
    let cleanup = |dir: &Path| {
        let _ = std::fs::remove_dir_all(dir);
    };

    // 1) 下载 + sha256 校验
    let zip_path = tmp_dir.join("download.zip");
    let actual = match download_to(&entry.url, &zip_path) {
        Ok(actual) => actual,
        Err(error) => {
            cleanup(&tmp_dir);
            return Err(error);
        }
    };
    if actual != entry.sha256.to_ascii_lowercase() {
        cleanup(&tmp_dir);
        return Err("文件校验失败，已取消安装".into());
    }

    // 2) 安全解压 + 结构归一 + 清单预校验（工具包放行脚本，皮肤包禁止）
    if let Err(error) = safe_extract(&zip_path, &tmp_dir, is_tool) {
        cleanup(&tmp_dir);
        return Err(error);
    }
    let _ = std::fs::remove_file(&zip_path);
    if let Err(error) =
        flatten_single_root(&tmp_dir, if is_tool { "tool.json" } else { "skin.json" })
    {
        cleanup(&tmp_dir);
        return Err(error);
    }
    if is_tool {
        // 工具包：校验清单 + id 一致性 + 工具名冲突（内置或其他插件占用）。
        let plugin = lingxi_tools::plugin::PluginTool::load(&tmp_dir).map_err(|error| {
            cleanup(&tmp_dir);
            format!("工具包校验失败：{error}")
        })?;
        if plugin.manifest().id != id {
            cleanup(&tmp_dir);
            return Err(format!(
                "工具包目录与清单 id 不一致：目录 {id}，清单 {}",
                plugin.manifest().id
            ));
        }
        let name = plugin.manifest().name.clone();
        let conflict = {
            let registry = state.tool_registry.safe_lock();
            let exists = registry.all_schemas().iter().any(|s| s.name == name);
            let is_self = state
                .plugin_tool_map
                .safe_lock()
                .get(&id)
                .is_some_and(|old| *old == name);
            exists && !is_self
        };
        if conflict {
            cleanup(&tmp_dir);
            return Err(format!("工具名 {name} 已被占用（内置工具或其他插件）"));
        }
    } else if let Err(error) = pet_skin::load_manifest_at(&tmp_dir, &id) {
        cleanup(&tmp_dir);
        return Err(format!("皮肤包校验失败：{error}"));
    }

    // 3) 落地：升级场景先备份旧目录到 .old，rename 成功后删除备份；
    //    rename 失败则把备份还原，保证旧版本继续可用。
    let final_dir = user_root.join(&id);
    let old_dir = user_root.join(format!(".old-{id}"));
    let had_old = final_dir.exists();
    if had_old {
        let _ = std::fs::remove_dir_all(&old_dir);
        if let Err(error) = std::fs::rename(&final_dir, &old_dir) {
            cleanup(&tmp_dir);
            return Err(format!("备份旧版本失败：{error}"));
        }
    }
    if let Err(error) = std::fs::rename(&tmp_dir, &final_dir) {
        if had_old {
            let _ = std::fs::rename(&old_dir, &final_dir);
        }
        cleanup(&tmp_dir);
        return Err(format!("安装失败：{error}"));
    }
    if had_old {
        let _ = std::fs::remove_dir_all(&old_dir);
    }

    // 4) 工具插件 → 同步注册表即可（Agent 下轮对话即可调用）；
    //    皮肤的升级目标正在使用中 → 重载并广播，桌宠立即换新版本。
    if is_tool {
        sync_plugin_tools(state.inner());
        return Ok(());
    }
    let (is_current, overrides, visible) = {
        let settings = state.backend.safe_lock();
        (
            settings.pet_skin == id,
            settings.pet_bubble_overrides.clone(),
            settings.pet_visible,
        )
    };
    if is_current {
        if let Ok(view) = pet_skin::view_for(&id, &overrides, visible) {
            let _ = app.emit("pet-config-changed", &view);
        }
    }
    Ok(())
}

/// 卸载用户皮肤。内置皮肤不可删除；删除使用中的皮肤时先回落默认皮肤，
/// 回落失败则中止删除，保证桌宠永远停留在可用皮肤上。
/// #[tauri::command(async)]：与另两个市场命令保持一致，磁盘 IO 也离开主线程。
#[tauri::command(async)]
pub(crate) fn market_uninstall(
    app: tauri::AppHandle,
    state: State<AppState>,
    id: String,
) -> Result<(), String> {
    if !pet_skin::valid_skin_id(&id) {
        return Err("id 非法".into());
    }
    // 先按皮肤查；不是皮肤再按工具插件查（工具没有「内置」形态）。
    let tool_dir = plugins_root().map(|root| root.join(&id));
    let is_tool = pet_skin::skin_source(&id).is_none()
        && tool_dir
            .as_ref()
            .is_some_and(|dir| dir.join(lingxi_tools::plugin::MANIFEST_FILE).is_file());
    if is_tool {
        let target = tool_dir.expect("已确认存在");
        let plugin = lingxi_tools::plugin::PluginTool::load(&target)
            .map_err(|error| format!("读取插件清单失败：{error}"))?;
        // 先摘注册表再删目录：删除失败时下次 sync 仍能找回。
        state
            .tool_registry
            .safe_lock()
            .remove(&plugin.manifest().name);
        state.plugin_tool_map.safe_lock().remove(&id);
        std::fs::remove_dir_all(&target).map_err(|error| format!("删除插件目录失败：{error}"))?;
        return Ok(());
    }
    match pet_skin::skin_source(&id) {
        Some(pet_skin::SKIN_SOURCE_USER) => {}
        Some(_) => return Err("内置皮肤不可删除".into()),
        None => return Err("皮肤不存在".into()),
    }
    let user_root = pet_skin::user_skins_dir().ok_or_else(|| "无法定位用户皮肤目录".to_string())?;
    let target = user_root.join(&id);
    if !target.starts_with(&user_root) {
        return Err("非法的皮肤路径".into());
    }

    // 使用中 → 先回落；只有回落成功才允许删目录，否则桌宠会指向已丢失的皮肤。
    let is_current = state.backend.safe_lock().pet_skin == id;
    if is_current {
        switch_to_default_skin(&app, &state)?;
    }

    std::fs::remove_dir_all(&target).map_err(|error| format!("删除皮肤目录失败：{error}"))?;
    Ok(())
}

/// 回落默认皮肤：完整复刻 `pet.rs::set_pet_skin` 的既有模式
/// （校验默认皮肤可加载 → 锁内改配置并持久化 → 解锁重建视图 → 广播）。
/// 与 market_install 末尾"尽力广播"不同，这里用 `?` 上抛：回落失败必须中止删除。
fn switch_to_default_skin(app: &tauri::AppHandle, state: &State<AppState>) -> Result<(), String> {
    let default_id = pet_skin::DEFAULT_SKIN_ID;
    // 先确认默认皮肤清单可加载（内置皮肤正常必然成功，双保险）。
    pet_skin::load_manifest(default_id)?;
    let (overrides, visible) = {
        let mut backend = state.backend.safe_lock();
        backend.pet_skin = default_id.to_string();
        crate::settings::persist_backend_settings(&backend)?;
        (backend.pet_bubble_overrides.clone(), backend.pet_visible)
    };
    let view = pet_skin::view_for(default_id, &overrides, visible)?;
    let _ = app.emit("pet-config-changed", &view);
    Ok(())
}

/// 已安装工具插件的展示视图。
#[derive(Serialize)]
pub(crate) struct ToolPluginView {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub author: String,
    pub version: String,
    pub description: String,
}

/// 已安装工具插件列表（市场「已安装」页签）。
#[tauri::command]
pub(crate) fn list_tool_plugins() -> Vec<ToolPluginView> {
    let root = plugins_root().unwrap_or_else(|| PathBuf::from("."));
    lingxi_tools::plugin::scan_plugins(&root)
        .into_iter()
        .map(|plugin| {
            let manifest = plugin.manifest();
            ToolPluginView {
                id: manifest.id.clone(),
                name: manifest.name.clone(),
                display_name: if manifest.display_name.is_empty() {
                    manifest.name.clone()
                } else {
                    manifest.display_name.clone()
                },
                author: manifest.author.clone(),
                version: manifest.version.clone(),
                description: manifest.description.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_entry() -> MarketEntry {
        MarketEntry {
            id: "demo-skin".into(),
            name: "演示皮肤".into(),
            author: "lingxi".into(),
            version: "1.0.0".into(),
            description: String::new(),
            url: "https://example.com/demo-skin-1.0.0.zip".into(),
            sha256: "0".repeat(64),
            size: Some(1024),
            thumbnail: String::new(),
            kind: "skin".into(),
        }
    }

    #[test]
    fn version_rank_orders() {
        assert!(version_newer("1.2.10", "1.2.9"));
        assert!(!version_newer("1.2.9", "1.2.10"));
        assert!(!version_newer("1.0.0", "1.0.0"));
        assert!(version_newer("2.0", "1.9.9"));
        assert!(!version_newer("1.0", "1.0.0"));
        assert!(version_newer("1.0.1", "1.0"));
    }

    #[test]
    fn sanitize_rejects_bad_entries() {
        assert!(sanitize_entry(demo_entry()).is_some());
        assert!(sanitize_entry(MarketEntry {
            id: "../evil".into(),
            ..demo_entry()
        })
        .is_none());
        assert!(sanitize_entry(MarketEntry {
            id: String::new(),
            ..demo_entry()
        })
        .is_none());
        assert!(sanitize_entry(MarketEntry {
            url: "ftp://x/a.zip".into(),
            ..demo_entry()
        })
        .is_none());
        assert!(sanitize_entry(MarketEntry {
            sha256: "abc".into(),
            ..demo_entry()
        })
        .is_none());
        assert!(sanitize_entry(MarketEntry {
            name: "  ".into(),
            ..demo_entry()
        })
        .is_none());
    }

    #[test]
    fn sanitize_clears_bad_thumbnail_only() {
        let mut entry = demo_entry();
        entry.thumbnail = "javascript:alert(1)".into();
        let cleaned = sanitize_entry(entry).expect("缩略图非法不应丢条目");
        assert!(cleaned.thumbnail.is_empty());
        let mut entry = demo_entry();
        entry.thumbnail = "https://cdn.example.com/preview.png".into();
        let kept = sanitize_entry(entry).expect("合法条目");
        assert_eq!(kept.thumbnail, "https://cdn.example.com/preview.png");
    }

    #[test]
    fn sanitize_rejects_unknown_kind() {
        assert!(sanitize_entry(MarketEntry {
            kind: "widget".into(),
            ..demo_entry()
        })
        .is_none());
        assert!(sanitize_entry(MarketEntry {
            kind: "tool".into(),
            ..demo_entry()
        })
        .is_some());
        // 缺省 kind = skin（旧清单向后兼容）
        let legacy = r#"{"entries":[{"id":"s","name":"n","version":"1","url":"https://e.com/a.zip","sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}]}"#;
        let catalog = parse_catalog(legacy).expect("旧清单应兼容");
        assert_eq!(catalog.entries[0].kind, "skin");
    }
}
