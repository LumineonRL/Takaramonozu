// Hand-rolled xoshiro128** because my google skills are so bad that I couldn't find a mature implementation
// which makes no sense there's no way that there's not an established library out there but who cares
// it takes 5 minutes to implement by hand. EDIT: Okay it took closer to an hour but whatever.
//
// See: <https://github.com/rust-random/rngs/blob/master/rand_xoshiro/src/xoshiro128starstar.rs>
// and <https://github.com/rust-random/rngs/blob/master/rand_xoshiro/src/splitmix64.rs>
// with a touch of <https://github.com/rust-random/rngs/blob/master/rand_xoshiro/src/common.rs>

const MASK64 = (1n << 64n) - 1n

const rotl = (x: number, k: number) => (x << k) | (x >>> (32 - k))

export function xoshiro128StarStar(seed: bigint): () => number {
    let x = BigInt.asUintN(64, seed)
    const words: number[] = []
    for (let i = 0; i < 2; i++) {
        x = (x + 0x9e3779b97f4a7c15n) & MASK64
        let z = x
        z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & MASK64
        z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & MASK64
        z ^= z >> 31n
        words.push(Number(z & 0xffffffffn), Number(z >> 32n))
    }

    let [s0, s1, s2, s3] = words as [number, number, number, number]
    return () => {
        const result = Math.imul(rotl(Math.imul(s1, 5), 7), 9) >>> 0
        const t = s1 << 9
        s2 ^= s0
        s3 ^= s1
        s1 ^= s2
        s0 ^= s3
        s2 ^= t
        s3 = rotl(s3, 11)
        return result
    }
}
