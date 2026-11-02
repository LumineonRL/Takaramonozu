// Tests that the first 8 draws of the hand-rolled xoshiro128** matches what the rand_xoshiro crate produces.

import { deepStrictEqual } from 'node:assert'
import { test } from 'node:test'

import { xoshiro128StarStar } from './rng.ts'

const VECTORS: [bigint, number[]][] = [
    [0n, [3737715805, 2584255861, 2876756834, 3286328325, 1553311962, 1625202774, 3260698944, 2754151956]],
    [12345n, [2314518269, 2498321016, 2055377852, 4042509560, 1267802836, 503974162, 1443322985, 3447162595]],
    [2n ** 64n - 1n, [477689756, 2493998634, 555695776, 607808419, 61340979, 301466976, 2478485284, 2565876330]],
]

test('matches rand_xoshiro', () => {
    for (const [seed, expected] of VECTORS) {
        const next = xoshiro128StarStar(seed)
        deepStrictEqual(
            Array.from(expected, () => next()),
            expected
        )
    }
})
