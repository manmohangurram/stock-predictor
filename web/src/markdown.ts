/**
 * Just enough markdown for a prompt.
 *
 * The prompts are written as markdown — headings, lists, bold, fenced JSON — and were being shown
 * in a `<pre>`, which is why they read like a config file. This renders the subset those files
 * actually use, after escaping: no prompt text can ever become markup.
 */

const escape = (s: string): string =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/** Inline: code first, so its contents are not touched by the emphasis rules. */
const inline = (raw: string): string => {
  const codes: string[] = [];
  let out = escape(raw).replace(/`([^`]+)`/g, (_, code: string) => {
    codes.push(code);
    return `\u0000${codes.length - 1}\u0000`;
  });
  out = out
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[\s(])\*([^*\n]+)\*/g, "$1<em>$2</em>")
    .replace(/(^|[\s(])_([^_\n]+)_/g, "$1<em>$2</em>")
    .replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, '<a href="$2" rel="noreferrer noopener">$1</a>');
  return out.replace(/\u0000(\d+)\u0000/g, (_, i: string) => `<code>${codes[Number(i)]}</code>`);
};

export function renderMarkdown(src: string): string {
  const lines = src.replace(/\r\n/g, "\n").split("\n");
  const out: string[] = [];
  let para: string[] = [];
  let list: { tag: "ul" | "ol"; items: string[] } | null = null;

  const flushPara = () => {
    if (para.length) {
      // A line break inside a paragraph is a soft break: the prompt is hard-wrapped for
      // editing, and the browser should reflow it to the width it actually has.
      out.push(`<p>${para.map(inline).join(" ")}</p>`);
      para = [];
    }
  };
  const flush = () => {
    flushPara();
    if (list) {
      out.push(
        `<${list.tag}>${list.items.map((i) => `<li>${inline(i)}</li>`).join("")}</${list.tag}>`,
      );
      list = null;
    }
  };
  /** The item marker of a line, if it is one. */
  const marker = (text: string): "ul" | "ol" | null => {
    if (/^\s*[-*+]\s+/.test(text)) return "ul";
    if (/^\s*\d+[.)]\s+/.test(text)) return "ol";
    return null;
  };

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    const fence = /^\s*```(\w*)\s*$/.exec(line);
    if (fence) {
      flush();
      const body: string[] = [];
      i += 1;
      while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) {
        body.push(lines[i]);
        i += 1;
      }
      out.push(`<pre class="md-code"><code>${escape(body.join("\n"))}</code></pre>`);
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      flush();
      const size = Math.min(heading[1].length + 2, 6);
      out.push(`<h${size}>${inline(heading[2])}</h${size}>`);
      continue;
    }
    if (/^\s*(---|\*\*\*|___)\s*$/.test(line)) {
      flush();
      out.push("<hr>");
      continue;
    }
    const quote = /^\s*>\s?(.*)$/.exec(line);
    if (quote) {
      flush();
      out.push(`<blockquote>${inline(quote[1])}</blockquote>`);
      continue;
    }
    const bullet = /^\s*[-*+]\s+(.*)$/.exec(line);
    const numbered = /^\s*\d+[.)]\s+(.*)$/.exec(line);
    if (bullet || numbered) {
      const tag = bullet ? "ul" : "ol";
      if (para.length) flush();
      if (!list || list.tag !== tag) {
        if (list) flush();
        list = { tag, items: [] };
      }
      list.items.push((bullet ?? numbered)![1]);
      continue;
    }
    if (!line.trim()) {
      // A prompt numbers its rules 1. 1. 1. and leaves a blank line between them. That is one
      // list written lazily, so it must not become seven lists each starting at one.
      const next = lines.slice(i + 1).find((l) => l.trim());
      flushPara();
      if (!(list && next && marker(next) === list.tag)) flush();
      continue;
    }
    // A wrapped line under a bullet belongs to that bullet, not to a new paragraph — otherwise
    // every number in the prompt's list closes the list and the next item starts again at one.
    if (list) {
      const last = list.items.length - 1;
      list.items[last] = `${list.items[last]} ${line.trim()}`;
      continue;
    }
    para.push(line);
  }
  flush();
  return out.join("\n");
}
