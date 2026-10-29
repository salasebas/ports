export class LastShown<T> {
	#read: () => T | null;
	#last: T | null = null;
	current = $derived.by(() => (this.#last = this.#read() ?? this.#last));

	constructor(read: () => T | null) {
		this.#read = read;
	}
}
