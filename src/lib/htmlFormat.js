// Аккуратный вид HTML в режиме «Код»: блочные теги — с новой строки и отступом.
// Переносы расставляем только вокруг блочных тегов, чтобы не добавить лишний пробел
// внутри текста или между inline-элементами. Содержимое pre/textarea/script/style
// не трогаем: там важны пробелы и переводы строк.

const BLOCK_TAGS = new Set([
  'html', 'head', 'body', 'div', 'p', 'blockquote', 'ul', 'ol', 'li', 'table', 'thead', 'tbody',
  'tfoot', 'tr', 'td', 'th', 'caption', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'hr', 'section',
  'article', 'header', 'footer', 'address', 'dl', 'dt', 'dd', 'figure', 'figcaption', 'main',
  'nav', 'aside', 'form', 'fieldset', 'pre',
]);
const VOID_TAGS = new Set([
  'area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'param', 'source',
  'track', 'wbr',
]);
const RAW_TAGS = new Set(['pre', 'textarea', 'script', 'style']);

// Разбиение на теги и текст: `>` внутри значения атрибута не должен закрывать тег.
function htmlTokens(html) {
  const tokens = [];
  let i = 0;
  while (i < html.length) {
    if (html[i] !== '<') {
      const next = html.indexOf('<', i);
      const end = next < 0 ? html.length : next;
      tokens.push({ tag: false, text: html.slice(i, end) });
      i = end;
      continue;
    }
    if (html.startsWith('<!--', i)) {
      const end = html.indexOf('-->', i + 4);
      const stop = end < 0 ? html.length : end + 3;
      tokens.push({ tag: false, text: html.slice(i, stop) });
      i = stop;
      continue;
    }
    let quote = null;
    let j = i + 1;
    while (j < html.length) {
      const ch = html[j];
      if (quote) {
        if (ch === quote) quote = null;
      } else if (ch === '"' || ch === "'") quote = ch;
      else if (ch === '>') break;
      j++;
    }
    const stop = Math.min(j + 1, html.length);
    tokens.push({ tag: true, text: html.slice(i, stop) });
    i = stop;
  }
  return tokens;
}

const tagName = (tag) => tag.match(/^<\/?\s*([a-zA-Z0-9-]+)/)?.[1]?.toLowerCase() ?? '';

export function formatHtml(html) {
  if (!html) return '';
  const tokens = htmlTokens(html);
  const lines = [];
  let line = '';
  let depth = 0;
  const indent = (level = depth) => '  '.repeat(Math.max(0, level));
  const flush = () => {
    if (line.trim()) lines.push(indent() + line.trim());
    line = '';
  };
  for (let i = 0; i < tokens.length; i++) {
    const token = tokens[i];
    if (!token.tag) {
      line += token.text.replace(/\s+/g, ' ');
      continue;
    }
    const name = tagName(token.text);
    const closing = token.text.startsWith('</');
    if (!closing && RAW_TAGS.has(name)) {
      let end = i + 1;
      while (end < tokens.length) {
        if (tokens[end].tag && tokens[end].text.startsWith('</') && tagName(tokens[end].text) === name) break;
        end++;
      }
      const inner = tokens.slice(i + 1, end).map((t) => t.text).join('').replace(/\s+$/, '');
      flush();
      lines.push(indent() + token.text);
      if (inner) lines.push(inner);
      if (end < tokens.length) lines.push(indent() + tokens[end].text);
      i = end;
      continue;
    }
    if (!BLOCK_TAGS.has(name)) {
      line += token.text;
      continue;
    }
    const selfClosing = VOID_TAGS.has(name) || /\/>\s*$/.test(token.text);
    flush();
    if (closing) {
      depth = Math.max(0, depth - 1);
      lines.push(indent() + token.text);
    } else {
      lines.push(indent() + token.text);
      if (!selfClosing) depth++;
    }
  }
  flush();
  return lines.join('\n');
}
