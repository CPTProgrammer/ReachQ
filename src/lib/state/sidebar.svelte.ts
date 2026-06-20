const STORAGE_KEY = 'reach-sidebar-width';
export const MIN_WIDTH = 160;
export const MAX_WIDTH = 600;
export const DEFAULT_WIDTH = 300;
export const COLLAPSED_WIDTH = 48;

function loadWidth(): number {
	try {
		const saved = localStorage.getItem(STORAGE_KEY);
		if (saved) {
			const n = parseInt(saved, 10);
			if (n >= MIN_WIDTH && n <= MAX_WIDTH) return n;
		}
	} catch {}
	return DEFAULT_WIDTH;
}

function saveWidth(w: number): void {
	try {
		localStorage.setItem(STORAGE_KEY, String(w));
	} catch {}
}

class SidebarState {
  #width = $state(loadWidth());

  get width() {
    return this.#width;
  }
  set width(value: number) {
    if (value < MIN_WIDTH) value = MIN_WIDTH;
    if (value > MAX_WIDTH) value = MAX_WIDTH;
    this.#width = value;
    saveWidth(value);
  }

  collapsed = $state(false);
  dragging = $state(false);
}

export const sidebarState = new SidebarState();
