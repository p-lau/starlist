// Whether the getting-started guide shows. It opens on a first visit and stays
// closed once dismissed; the header's Guide button brings it back.

const KEY = 'starlist.guide.dismissed';

function dismissed(): boolean {
	try {
		return localStorage.getItem(KEY) === '1';
	} catch {
		return false;
	}
}

export const guide = $state({ open: typeof localStorage !== 'undefined' && !dismissed() });

export function closeGuide() {
	guide.open = false;
	try {
		localStorage.setItem(KEY, '1');
	} catch {
		// Private windows can refuse storage; the guide just shows again next time.
	}
}

export function toggleGuide() {
	if (guide.open) closeGuide();
	else guide.open = true;
}
