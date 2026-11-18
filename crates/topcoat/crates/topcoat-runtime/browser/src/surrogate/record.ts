import { dehydrate } from "../expression/dehydrate";
import type { DehydratedSurrogate } from "../expression/serialized";
import { cloneValue } from "./ref";

/**
 * The browser representation of a `#[record]` struct. Named properties let
 * generated expressions access fields directly, as in `order.id`.
 */
export class Record {
	[field: string]: unknown;

	constructor(fields: Iterable<readonly [string, unknown]>) {
		// Defining the fields avoids setters such as `__proto__`.
		for (const [name, value] of fields) {
			Object.defineProperty(this, name, { value, enumerable: true });
		}
	}

	clone(): Record {
		return new Record(
			Object.entries(this).map(([name, value]) => [name, cloneValue(value)]),
		);
	}

	dehydrate(): DehydratedSurrogate {
		return {
			t: "Record",
			v: Object.fromEntries(
				Object.entries(this).map(([name, value]) => [name, dehydrate(value)]),
			),
		};
	}
}
