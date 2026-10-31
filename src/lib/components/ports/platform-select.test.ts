import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, it, vi } from "vitest";
import PlatformSelect from "./platform-select.svelte";

const platform = vi.hoisted(() => ({ isMac: false }));
vi.mock("$lib/platform", () => ({
	get isMac() {
		return platform.isMac;
	},
}));

const options = [
	{ value: "one", label: "One" },
	{ value: "two", label: "Two" },
];
let view: ReturnType<typeof mount> | undefined;

afterEach(() => {
	if (view) void unmount(view);
	view = undefined;
	document.body.replaceChildren();
});

function show(mac: boolean) {
	platform.isMac = mac;
	view = mount(PlatformSelect, { target: document.body, props: { value: "two", options, id: "choice" } });
	flushSync();
}

it("uses the system menu on macOS", () => {
	show(true);
	expect(document.querySelector<HTMLSelectElement>("select#choice")?.value).toBe("two");
});

it("uses the app's dropdown elsewhere and shows the current choice", () => {
	show(false);
	expect(document.querySelector("select")).toBeNull();
	expect(document.querySelector("button#choice")?.textContent).toContain("Two");
});
