//! Minimal, safe markdown renderer for assistant messages and tool results
//! (design 01 §2.2). Everything is HTML-escaped first; code blocks get a
//! Copy button (`.md-copy`, handled via event delegation by the chat view).

import { t } from '$lib/state/i18n.svelte';

function escapeHtml(s: string): string {
	return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function inline(src: string): string {
	let s = escapeHtml(src);
	// Inline code first so its contents are not re-processed.
	s = s.replace(/`([^`]+)`/g, '\u0000$1\u0000');
	s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
	s = s.replace(/(^|[^*])\*([^*\n]+)\*/g, '$1<em>$2</em>');
	s = s.replace(/~~([^~]+)~~/g, '<del>$1</del>');
	s = s.replace(
		/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g,
		'<a href="$2" target="_blank" rel="noreferrer">$1</a>'
	);
	s = s.replace(/\u0000([^\u0000]+)\u0000/g, '<code class="md-inline">$1</code>');
	return s;
}

export function renderMarkdown(src: string): string {
	const lines = src.replace(/\r\n/g, '\n').split('\n');
	let html = '';
	let i = 0;
	let para: string[] = [];
	let list: { ordered: boolean; items: string[] } | null = null;

	const flushPara = () => {
		if (para.length) {
			html += `<p>${para.map(inline).join('<br>')}</p>`;
			para = [];
		}
	};
	const flushList = () => {
		if (list) {
			const tag = list.ordered ? 'ol' : 'ul';
			html += `<${tag}>${list.items.map((it) => `<li>${inline(it)}</li>`).join('')}</${tag}>`;
			list = null;
		}
	};

	while (i < lines.length) {
		const line = lines[i];

		const fence = line.match(/^```(\w*)\s*$/);
		if (fence) {
			flushPara();
			flushList();
			const lang = fence[1];
			const buf: string[] = [];
			i++;
			while (i < lines.length && !/^```\s*$/.test(lines[i])) {
				buf.push(lines[i]);
				i++;
			}
			i++; // skip closing fence (or EOF)
			const raw = buf.join('\n');
			html +=
				`<div class="md-code"><div class="md-code-head"><span>${escapeHtml(lang) || 'code'}</span>` +
				`<button type="button" class="md-copy" data-code="${encodeURIComponent(raw)}">${escapeHtml(t('agent.copy'))}</button></div>` +
				`<pre><code>${escapeHtml(raw)}</code></pre></div>`;
			continue;
		}

		const heading = line.match(/^(#{1,4})\s+(.*)$/);
		if (heading) {
			flushPara();
			flushList();
			const level = heading[1].length;
			html += `<h${level}>${inline(heading[2])}</h${level}>`;
			i++;
			continue;
		}

		if (/^\s*(-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
			flushPara();
			flushList();
			html += '<hr>';
			i++;
			continue;
		}

		const quote = line.match(/^>\s?(.*)$/);
		if (quote) {
			flushPara();
			flushList();
			html += `<blockquote>${inline(quote[1])}</blockquote>`;
			i++;
			continue;
		}

		const ul = line.match(/^\s*[-*+]\s+(.*)$/);
		const ol = line.match(/^\s*\d+[.)]\s+(.*)$/);
		if (ul || ol) {
			flushPara();
			const ordered = !!ol;
			const item = (ul ?? ol)![1];
			if (list && list.ordered === ordered) {
				list.items.push(item);
			} else {
				flushList();
				list = { ordered, items: [item] };
			}
			i++;
			continue;
		}
		flushList();

		if (line.trim() === '') {
			flushPara();
		} else {
			para.push(line);
		}
		i++;
	}
	flushPara();
	flushList();
	return html;
}
