// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

// StateMap[activity_ok_bit][activity_fail_bit]
// - StateMap[0][0] = 'Unrecorded'
// - StateMap[0][1] = 'Failure'
// - StateMap[1][0] = 'Ok'
// - StateMap[1][1] = 'Unstable' ///
// Unstable means that both `Failure` and `Ok` happens in 1 tick
const StateMap = [
	['Unrecorded', 'Failure'],
	['Ok', 'Unstable']
] as const

export type ActivityState = (typeof StateMap)[number][number]
export type State = ActivityState

const ACTIVITY_WINDOW_LENGTH = 288
const BUCKET_MS = 5 * 60 * 1000

// Bucket `i` is bit `i % 8` of byte `i / 8`, LSB-first. Planes come back empty
// when the item recorded nothing in the window, which reads as all-zero.
function bucket_bit(plane: number[], bucket: number): number {
	return ((plane[bucket >> 3] ?? 0) >> (bucket & 7)) & 1
}

/**
 * The window's 288 five-minute buckets, oldest first, ending at
 * `activity_window_end`.
 *
 * The planes are a ring, not a list: a bucket's position is its absolute
 * 5-minute tick mod 288, so position 0 is always 00:00–00:05 UTC. One lap is
 * exactly 24 h, so the window straddles two UTC dates and splits right after
 * `end % 288` — positions past it haven't come round yet and are a day older:
 *
 *   out[]:  145 … 287 │ 0 … 144   ← bucket positions when end % 288 == 144
 *           yesterday │ today       (oldest first; newest bucket = 144)
 *
 */
export function parse_activity_window(
	activity_ok: number[],
	activity_fail: number[],
	activity_window_end: Date
): { time: Date; state: State }[] {
	const end_tick = Math.floor(activity_window_end.getTime() / BUCKET_MS)
	const out: { time: Date; state: State }[] = []
	for (let age = ACTIVITY_WINDOW_LENGTH - 1; age >= 0; age--) {
		const tick = end_tick - age
		const bucket =
			((tick % ACTIVITY_WINDOW_LENGTH) + ACTIVITY_WINDOW_LENGTH) % ACTIVITY_WINDOW_LENGTH
		out.push({
			time: new Date(tick * BUCKET_MS),
			state: StateMap[bucket_bit(activity_ok, bucket)][bucket_bit(activity_fail, bucket)]
		})
	}
	return out
}
