import test from 'node:test';
import assert from 'node:assert/strict';
import { formatHtml } from '../src/lib/htmlFormat.js';

test('block tags get their own line and indentation', () => {
  const html = '<div>Dear <b>Robert</b>,</div><div><br></div><blockquote><p>Hi</p></blockquote>';
  assert.equal(
    formatHtml(html),
    [
      '<div>',
      '  Dear <b>Robert</b>,',
      '</div>',
      '<div>',
      '  <br>',
      '</div>',
      '<blockquote>',
      '  <p>',
      '    Hi',
      '  </p>',
      '</blockquote>',
    ].join('\n'),
  );
});

test('inline elements keep their spacing', () => {
  assert.equal(formatHtml('<p>Hello <b>world</b>!</p>'), ['<p>', '  Hello <b>world</b>!', '</p>'].join('\n'));
  assert.equal(formatHtml('<span>a</span><span>b</span>'), '<span>a</span><span>b</span>');
});

test('a greater-than sign inside an attribute does not close the tag', () => {
  assert.equal(formatHtml('<div title="a > b">x</div>'), ['<div title="a > b">', '  x', '</div>'].join('\n'));
});

test('preformatted text is left exactly as written', () => {
  assert.equal(formatHtml('<pre>a\n  b</pre>'), '<pre>\na\n  b\n</pre>');
});
