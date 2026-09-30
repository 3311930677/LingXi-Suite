// 轻量 Markdown 渲染器（B2-1）：
// - 安全：全部用 DOM API 构建（createElement/textContent），不拼接 innerHTML；
//   行内标记只识别白名单结构，链接不产生真实导航（点击复制 URL），
//   避免 WebView2 内导航劫持面板。
// - 覆盖：围栏代码块（语言标签 + 复制按钮）、标题、有序/无序列表、
//   引用、分隔线、段落；行内 `code`、**粗体**、*斜体*、~~删除线~~、[链接](url)。
// - 主面板与桌宠对话共用（pet.html / index.html 均在 app.js 之前引入）。

(function () {
  "use strict";

  /// 行内标记 → DOM 节点序列。
  function renderInline(text) {
    const nodes = [];
    // 解析顺序：行内代码最先（其内容不再二次解析）→ 链接 → 粗/斜/删除线。
    const pattern =
      /(`[^`]+`)|(\[[^\]]+\]\((?:https?:\/\/)[^\s)]+\))|(\*\*[^*]+\*\*)|(\*[^*]+\*)|(~~[^~]+~~)/g;
    let last = 0;
    let match;
    while ((match = pattern.exec(text)) !== null) {
      if (match.index > last) {
        nodes.push(document.createTextNode(text.slice(last, match.index)));
      }
      const token = match[0];
      if (token.startsWith("`")) {
        const code = document.createElement("code");
        code.className = "md-inline-code";
        code.textContent = token.slice(1, -1);
        nodes.push(code);
      } else if (token.startsWith("[")) {
        const closeParen = token.lastIndexOf("](");
        const label = token.slice(1, closeParen);
        const url = token.slice(closeParen + 2, -1);
        // 不渲染可导航 <a>：WebView2 内导航会劫持面板；点击复制链接。
        const link = document.createElement("span");
        link.className = "md-link";
        link.textContent = label;
        link.title = url + "（点击复制链接）";
        link.addEventListener("click", () => copyText(url, link));
        nodes.push(link);
      } else if (token.startsWith("**")) {
        const bold = document.createElement("strong");
        bold.textContent = token.slice(2, -2);
        nodes.push(bold);
      } else if (token.startsWith("~~")) {
        const del = document.createElement("del");
        del.textContent = token.slice(2, -2);
        nodes.push(del);
      } else {
        const em = document.createElement("em");
        em.textContent = token.slice(1, -1);
        nodes.push(em);
      }
      last = match.index + token.length;
    }
    if (last < text.length) {
      nodes.push(document.createTextNode(text.slice(last)));
    }
    return nodes;
  }

  function copyText(text, tipTarget) {
    const done = () => {
      if (!tipTarget) return;
      const previous = tipTarget.title;
      tipTarget.title = "已复制";
      setTimeout(() => {
        tipTarget.title = previous;
      }, 1200);
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(done, () => fallbackCopy(text, done));
    } else {
      fallbackCopy(text, done);
    }
  }

  /// WebView2 权限不足时 navigator.clipboard 会静默失败：退回 execCommand。
  function fallbackCopy(text, done) {
    const area = document.createElement("textarea");
    area.value = text;
    area.style.cssText = "position:fixed;opacity:0;";
    document.body.appendChild(area);
    area.select();
    try {
      document.execCommand("copy");
      done();
    } catch {
      /* 全部失败时静默 */
    }
    area.remove();
  }

  function appendInline(parent, text) {
    for (const node of renderInline(text)) parent.append(node);
  }

  /// 围栏代码块：头部（语言标签 + 复制按钮）+ 滚动 <pre>。
  function buildCodeBlock(language, codeText) {
    const wrap = document.createElement("div");
    wrap.className = "md-codeblock";
    const head = document.createElement("div");
    head.className = "md-code-head";
    const lang = document.createElement("span");
    lang.className = "md-code-lang";
    lang.textContent = language || "text";
    const copy = document.createElement("button");
    copy.type = "button";
    copy.className = "md-code-copy";
    copy.textContent = "复制";
    copy.addEventListener("click", () => {
      copyText(codeText, copy);
      copy.textContent = "已复制";
      setTimeout(() => {
        copy.textContent = "复制";
      }, 1200);
    });
    head.append(lang, copy);
    const pre = document.createElement("pre");
    pre.className = "md-code";
    pre.textContent = codeText;
    wrap.append(head, pre);
    return wrap;
  }

  /// 把 markdown 文本渲染进 container（清空原有内容）。
  /// 极限保护：超长输入截断（防止一次性巨量渲染卡死 UI 线程）。
  function renderMarkdownInto(container, text) {
    const MAX_CHARS = 60_000;
    let source = String(text || "");
    let truncated = false;
    if (source.length > MAX_CHARS) {
      source = source.slice(0, MAX_CHARS);
      truncated = true;
    }
    container.replaceChildren();
    const lines = source.split("\n");
    let paragraph = null;
    let list = null; // { element, ordered }
    let quote = null;

    const flushParagraph = () => {
      paragraph = null;
    };
    const flushList = () => {
      list = null;
    };
    const flushQuote = () => {
      quote = null;
    };
    const flushAll = () => {
      flushParagraph();
      flushList();
      flushQuote();
    };

    for (let i = 0; i < lines.length; i += 1) {
      const line = lines[i];
      const trimmed = line.trim();

      // 围栏代码块（``` 开头，闭合或到文末）。
      const fence = trimmed.match(/^```(\w*)\s*$/);
      if (fence) {
        flushAll();
        const body = [];
        let j = i + 1;
        for (; j < lines.length && !lines[j].trim().startsWith("```"); j += 1) {
          body.push(lines[j]);
        }
        container.append(buildCodeBlock(fence[1], body.join("\n")));
        i = j; // 循环 i+=1 会跳过闭合围栏
        continue;
      }

      if (!trimmed) {
        flushAll();
        continue;
      }

      // 标题（# ~ ####）。
      const heading = trimmed.match(/^(#{1,4})\s+(.*)$/);
      if (heading) {
        flushAll();
        const level = heading[1].length;
        const head = document.createElement("h" + level);
        head.className = "md-heading md-h" + level;
        appendInline(head, heading[2]);
        container.append(head);
        continue;
      }

      // 分隔线。
      if (/^(-{3,}|\*{3,})$/.test(trimmed)) {
        flushAll();
        const hr = document.createElement("hr");
        hr.className = "md-hr";
        container.append(hr);
        continue;
      }

      // 引用。
      const quoteMatch = line.match(/^\s*>\s?(.*)$/);
      if (quoteMatch) {
        flushParagraph();
        flushList();
        if (!quote) {
          quote = document.createElement("blockquote");
          quote.className = "md-quote";
          container.append(quote);
        }
        const row = document.createElement("div");
        appendInline(row, quoteMatch[1]);
        quote.append(row);
        continue;
      }
      flushQuote();

      // 列表（支持嵌套一层：两空格缩进）。
      const unordered = line.match(/^(\s*)[-*+]\s+(.*)$/);
      const ordered = line.match(/^(\s*)\d+[.、]\s+(.*)$/);
      if (unordered || ordered) {
        flushParagraph();
        const indent = (unordered || ordered)[1].length >= 2;
        const wantedOrdered = Boolean(ordered);
        if (!list || list.ordered !== wantedOrdered) {
          list = {
            ordered: wantedOrdered,
            element: document.createElement(wantedOrdered ? "ol" : "ul"),
          };
          list.element.className = "md-list";
          container.append(list.element);
        }
        const item = document.createElement("li");
        if (indent) item.className = "md-li-nested";
        appendInline(item, (unordered || ordered)[2]);
        list.element.append(item);
        continue;
      }
      flushList();

      // 普通段落行（同一连续行合并为一段）。
      if (!paragraph) {
        paragraph = document.createElement("p");
        paragraph.className = "md-p";
        container.append(paragraph);
      } else {
        paragraph.append(document.createElement("br"));
      }
      appendInline(paragraph, line);
    }

    if (truncated) {
      const note = document.createElement("p");
      note.className = "md-truncated";
      note.textContent = "（内容过长，已截断显示）";
      container.append(note);
    }
  }

  window.renderMarkdownInto = renderMarkdownInto;
  window.owoCopyText = copyText;
})();
