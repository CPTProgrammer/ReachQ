/**
 * Resize a textarea to fit its content. Call on input and after programmatic
 * value changes. Restores overflowY to the stylesheet value when done, so a
 * CSS `overflow-y: hidden` stays in effect (no scrollbar, uncapped height);
 * without it the UA-default `auto` shows a scrollbar only once a CSS
 * max-height clamps the height. Height accounts for border-box borders plus
 * 1px against fractional-rounding overflow.
 */
export function autogrowTextarea(el: HTMLTextAreaElement | null | undefined): void {
	if (!el) return;
	el.style.overflowY = 'hidden';
	el.style.height = 'auto';
	const borders = el.offsetHeight - el.clientHeight;
	el.style.height = `${el.scrollHeight + borders + 1}px`;
	el.style.overflowY = '';
}
