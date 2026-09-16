//! Shared stick-to-bottom ("follow") state for the chat view. Owned by
//! AgentChat (ResizeObserver re-pins on size change) and set by AgentComposer
//! plus the chat's own actions (sending always jumps to the bottom). Exit is
//! gesture-based (wheel / touch / scrollbar drag / up-keys); re-entry is
//! position-based: scrolling down into the hysteresis zone near the bottom
//! re-engages follow.

let following = $state(true);

export function getFollowing(): boolean {
	return following;
}

export function setFollowing(v: boolean): void {
	following = v;
}
