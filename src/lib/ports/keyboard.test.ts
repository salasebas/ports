import { describe, expect, it } from "vitest";
import { isEditable, listCommand, step, typedText } from "./keyboard";

const key = (name: string, modifiers: Partial<KeyboardEventInit> = {}) =>
	new KeyboardEvent("keydown", { key: name, ...modifiers });

describe("list keys", () => {
	it.each([
		["ArrowDown", {}, "next"],
		["ArrowUp", {}, "previous"],
		["Enter", {}, "details"],
		["o", { metaKey: true }, "open"],
		["Backspace", { metaKey: true }, "stop"],
	] as const)("maps %s on macOS", (name, modifiers, command) => {
		expect(listCommand(key(name, modifiers), true)).toBe(command);
	});

	it("uses Ctrl instead of ⌘ on Linux", () => {
		expect(listCommand(key("o", { ctrlKey: true }), false)).toBe("open");
		expect(listCommand(key("Backspace", { ctrlKey: true }), false)).toBe("stop");
		expect(listCommand(key("o", { metaKey: true }), false)).toBeNull();
	});

	it("leaves other modifier combinations alone", () => {
		expect(listCommand(key("o", { metaKey: true, shiftKey: true }), true)).toBeNull();
		expect(listCommand(key("ArrowDown", { altKey: true }), true)).toBeNull();
		expect(listCommand(key("Enter", { metaKey: true }), true)).toBeNull();
	});

	it("sends typed characters to the search, but not Space or shortcuts", () => {
		expect(typedText(key("3"))).toBe("3");
		expect(typedText(key("W", { shiftKey: true }))).toBe("W");
		expect(typedText(key(" "))).toBeNull();
		expect(typedText(key("c", { metaKey: true }))).toBeNull();
		expect(typedText(key("Tab"))).toBeNull();
	});

	it("tells text fields from controls", () => {
		const text = document.createElement("input");
		const checkbox = Object.assign(document.createElement("input"), { type: "checkbox" });
		expect(isEditable(text)).toBe(true);
		expect(isEditable(document.createElement("textarea"))).toBe(true);
		expect(isEditable(checkbox)).toBe(false);
		expect(isEditable(document.createElement("button"))).toBe(false);
		expect(isEditable(null)).toBe(false);
	});

	it("moves within the list and starts at the first row", () => {
		const ids = ["a", "b", "c"];
		expect(step(ids, null, 1)).toBe("a");
		expect(step(ids, "a", 1)).toBe("b");
		expect(step(ids, "c", 1)).toBe("c");
		expect(step(ids, "a", -1)).toBe("a");
		expect(step(ids, "gone", -1)).toBe("a");
		expect(step([], "a", 1)).toBeNull();
	});
});
