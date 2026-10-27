export const isMac = navigator.userAgent.includes("Mac");

export const fileManagerName = isMac ? "Finder" : "Files";

const modifierLabels: [string, string][] = isMac
	? [
			["control", "⌃"],
			["alt", "⌥"],
			["shift", "⇧"],
			["super", "⌘"],
		]
	: [
			["control", "Ctrl"],
			["alt", "Alt"],
			["shift", "Shift"],
			["super", "Super"],
		];

const keyLabels: Record<string, string> = {
	ArrowUp: "↑",
	ArrowDown: "↓",
	ArrowLeft: "←",
	ArrowRight: "→",
	Enter: "↩",
	Backspace: "⌫",
	Delete: "⌦",
	Escape: "Esc",
	Tab: "⇥",
	Backquote: "`",
	Backslash: "\\",
	BracketLeft: "[",
	BracketRight: "]",
	Comma: ",",
	Equal: "=",
	Minus: "-",
	Period: ".",
	Quote: "'",
	Semicolon: ";",
	Slash: "/",
};

/**
 * The keys of a shortcut as `src-tauri/src/shortcut.rs` returns it, in the
 * platform's order: `alt+super+KeyP` is ⌥ ⌘ P on macOS.
 */
export function shortcutKeys(shortcut: string): string[] {
	const parts = shortcut.split("+");
	const code = parts.pop() ?? "";
	const modifiers = parts.map((part) => part.toLowerCase());
	const key = code.replace(/^(Key|Digit)(?=.$)/, "");
	return [
		...modifierLabels.filter(([name]) => modifiers.includes(name)).map(([, label]) => label),
		keyLabels[key] ?? key,
	];
}

/**
 * The shortcut for a key press, by physical key so it matches what the
 * backend registers; null while only modifiers are down.
 */
export function shortcutFor(event: KeyboardEvent): string | null {
	if (/^(Shift|Control|Alt|Meta|OS)(Left|Right)?$/.test(event.code)) return null;
	const modifiers = [
		event.ctrlKey && "Control",
		event.altKey && "Alt",
		event.shiftKey && "Shift",
		event.metaKey && "Super",
	].filter(Boolean);
	return [...modifiers, event.code].join("+");
}
