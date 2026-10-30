import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { toast } from "svelte-sonner";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { rules, type RulesChange } from "$lib/rules.svelte";
import AutoCleanupSettings from "./auto-cleanup-settings.svelte";
import FilterRules from "./filter-rules.svelte";

let view: ReturnType<typeof mount> | undefined;
let refused = false;

beforeEach(() => {
	refused = false;
	mockIPC((command, args) => {
		if (command === "auto_cleanup_log") return [];
		if (command !== "update_rules") return undefined;
		if (refused) throw "Could not save the rules";
		const { change } = args as { change: RulesChange };
		const current = rules.current;
		const next = { ...current.rules };
		if (change.kind === "filters") Object.assign(next, { include: change.include, exclude: change.exclude });
		if (change.kind === "unhide") next.hidden = next.hidden.filter((entry) => entry !== change.entry);
		if (change.kind === "autoCleanup") next.autoCleanup = { policy: change.policy, paused: change.paused };
		return {
			rules: { revision: current.revision + 1, rules: next },
			snapshot: { revision: 1, servers: [], system: {} },
		};
	});
});

afterEach(() => {
	if (view) void unmount(view);
	view = undefined;
	document.body.replaceChildren();
	clearMocks();
	vi.restoreAllMocks();
});

function show(component: typeof FilterRules | typeof AutoCleanupSettings) {
	view = mount(component, { target: document.body, props: { id: "test" } });
	flushSync();
}

const field = (selector: string) => document.querySelector<HTMLInputElement>(selector)!;

function type(input: HTMLInputElement, text: string) {
	input.value = text;
	input.dispatchEvent(new Event("input", { bubbles: true }));
	flushSync();
}

const submit = (form: Element) => form.dispatchEvent(new SubmitEvent("submit", { bubbles: true, cancelable: true }));

it("confirms a saved port range", async () => {
	const confirmed = vi.spyOn(toast, "success");
	show(FilterRules);
	type(field("#test-range-start"), "3000");
	type(field('[aria-label="Last port"]'), "9999");
	submit(field("#test-range-start").form!);
	await vi.waitFor(() => expect(confirmed).toHaveBeenCalledWith("Port range saved"));
});

it("says every port is listed once the range is cleared", async () => {
	const confirmed = vi.spyOn(toast, "success");
	show(FilterRules);
	submit(field("#test-range-start").form!);
	await vi.waitFor(() => expect(confirmed).toHaveBeenCalledWith("Showing every port"));
});

it("confirms an added rule, and only when it was saved", async () => {
	const confirmed = vi.spyOn(toast, "success");
	show(FilterRules);
	type(field("#test-pattern"), "storybook");
	submit(field("#test-pattern").form!);
	await vi.waitFor(() => expect(confirmed).toHaveBeenCalledWith("Rule added"));

	confirmed.mockClear();
	refused = true;
	type(field("#test-pattern"), "vite");
	submit(field("#test-pattern").form!);
	await vi.waitFor(() => expect(document.body.textContent).toContain("Could not save the rules"));
	expect(confirmed).not.toHaveBeenCalled();
});

it("confirms a removed rule", async () => {
	const confirmed = vi.spyOn(toast, "success");
	rules.apply({
		revision: rules.current.revision + 1,
		rules: { ...rules.current.rules, exclude: [{ field: "process", pattern: "node" }] },
	});
	show(FilterRules);
	document.querySelector<HTMLButtonElement>('[aria-label="Remove the rule node"]')!.click();
	await vi.waitFor(() => expect(confirmed).toHaveBeenCalledWith("Rule removed"));
});

it("confirms a server shown again", async () => {
	const confirmed = vi.spyOn(toast, "success");
	rules.apply({
		revision: rules.current.revision + 1,
		rules: { ...rules.current.rules, exclude: [], hidden: ["port:5000"] },
	});
	show(FilterRules);
	document.querySelector<HTMLButtonElement>('[aria-label="Show :5000 again"]')!.click();
	await vi.waitFor(() => expect(confirmed).toHaveBeenCalledWith("Showing :5000 again"));
});

it("confirms turning automatic clean up on and off", async () => {
	const confirmed = vi.spyOn(toast, "success");
	show(AutoCleanupSettings);
	const scope = document.querySelector<HTMLSelectElement>("#test-auto-scope")!;
	scope.value = "deleted";
	scope.dispatchEvent(new Event("change", { bubbles: true }));
	await vi.waitFor(() => expect(confirmed).toHaveBeenCalledWith("Automatic clean up on"));
	flushSync();
	scope.value = "off";
	scope.dispatchEvent(new Event("change", { bubbles: true }));
	await vi.waitFor(() => expect(confirmed).toHaveBeenCalledWith("Automatic clean up off"));
});
