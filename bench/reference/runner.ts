/// <reference path="./globals.d.ts" />


// Headless driver for burrito's sim so teams can be made from the command-line instead of
// in the frontend.
// Reads a spec from `bench/specs`, calls the Context/Live parts of the simulator and returns the result.
// No game logic changes here at all. It literally just strips out any frontend components just to run
// the simulator and nothing else. And adds seeded RNG to make results more consistent without affecting performance.
//
// `cd bench/reference && npm ci && npm run build`
// `node bench/reference/dist/runner.mjs <spec.json> [trials]

import { existsSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { cards, charts, initDatabase, sises } from './sif-team-simulator/src/database'
import { Difficulty } from './sif-team-simulator/src/database/Chart'
import { Context } from './sif-team-simulator/src/core/simulation/Context'
import { Live } from './sif-team-simulator/src/core/simulation/Live'
import { calculateTeamStat } from './sif-team-simulator/src/core/stats'
import { Team } from './sif-team-simulator/src/core/Team'
import { CenterSkill } from './sif-team-simulator/src/database/Center'

import { xoshiro128StarStar } from './rng'

function findSimRoot() {
    let dir = dirname(fileURLToPath(import.meta.url))
    for (let i = 0; i < 8; i++) {
        const candidate = join(dir, 'sif-team-simulator')
        if (existsSync(candidate)) return candidate
        dir = dirname(dir)
    }
    throw new Error("can't find sif-team-simulator")
}

const SIM_ROOT = findSimRoot()

const realFetch = globalThis.fetch
globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const match = /^\/database\/(.+)$/.exec(url)
    if (!match) return realFetch(input as never, init)

    const text = readFileSync(join(SIM_ROOT, 'public/database', `${match[1]}`), 'utf8')
    return Promise.resolve({
        ok: true,
        status: 200,
        json: () => Promise.resolve(JSON.parse(text)),
        text: () => Promise.resolve(text),
    } as Response)
}) as typeof fetch

type SpecMember = {
    cardId: number
    level: number
    skillLevel: number
    // Burrito implements SIS differently than the Rust sim does and doesn't use
    // ids, so the design decision was to make it so that Burrito needs to read Rust's SIS IDs.
    sis: { id: number; type: string; value: number }[]
    accessory?: { id: string; level: number }
}

type Spec = {
    name: string
    chartId: string
    difficulty: number
    mode: string
    members: SpecMember[]
    guestCenter: CenterSkill
    memoryGalleryBonus: number[]
    noteSpeed: number
    tapScoreBonus: number
    skillChanceBonus: number
    skillChanceReduction: number
    accuracy: number
    trials: number
    seed?: number
}

const specPath = process.argv[2]
if (!specPath) throw new Error('usage: runner <spec.json> [trials]')
const spec: Spec = JSON.parse(readFileSync(specPath, 'utf8'))
const trials = process.argv[3] ? Number(process.argv[3]) : spec.trials

await initDatabase(spec.difficulty as Difficulty)

const chart = charts.get(spec.chartId)
if (!chart) throw new Error(`chart ${spec.chartId} missing at difficulty ${spec.difficulty}`)

// SIS needs special handling. This basically takes a SIS as the specs lay it out by id
// and converts it into something usable by Burrito's sim.
//
// Burrito's SIS table looks like this:
// ```
// ['Charm', { type: 'score', value: 2.5 }],
// ['Ring',  { type: 'self',  value: 0.1 }],
// ```
// So this is an intermediate step to turn something like this:
//
// ```
// slot 0: [{id: 157, name: 'Smile Dolphin Petit', type: 'self', value: 0.13}]
// ```
// into 
// ```
// 'spec:0:0:157' -> { type: 'self', value: 0.13 }
// ```
// which allows directly passing the value into burrito.
const team = spec.members.map((member, slot) => {
    if (!cards.has(member.cardId)) throw new Error(`card ${member.cardId} missing`)

    const sisNames = member.sis.map((sis, k) => {
        const name = `spec:${slot}:${k}:${sis.id}`
        sises.set(name, { type: sis.type as never, value: sis.value })
        return name
    })

    return {
        card: { id: member.cardId, level: member.level, sl: member.skillLevel },
        sisNames,
        accessory: member.accessory,
    }
}) as unknown as Team

// Deterministic RNG. Burrito previously did Math.random() on every RNG call, but that makes
// comparisons hard because two runs with the same team on the same song would produce different results.
//
// This is xoshiro128** seeded through SplitMix64. See `rng.ts`. Needed something A) Fast, can't be slower than Math.random()
// and B) what the Rust sim uses. The Rust sim uses `rand_xoshiro` so I implemented that for the typescript version too.
// I went looking for a mature implementation of xoshiro but couldn't find one so I had to hand-roll it and thought that was weird
// I found this <https://github.com/joeltg/xoshiro-js> but it looks quite immature. Plus it was a good way to practice ts.
if (spec.seed !== undefined && spec.seed !== null) {
    const next = xoshiro128StarStar(BigInt(spec.seed))
    Math.random = () => next() / 2 ** 32
}

const performance = {
    perfect: spec.accuracy,
    great: 1 - spec.accuracy,
    good: 0,
    bad: 0,
    miss: 0,
    overwrites: {},
}

const context = new Context(
    team,
    spec.mode,
    spec.memoryGalleryBonus,
    spec.guestCenter,
    spec.chartId,
    performance,
    spec.noteSpeed,
    spec.tapScoreBonus,
    spec.skillChanceBonus,
    spec.skillChanceReduction,
    false
)

const teamStat = calculateTeamStat(team, spec.memoryGalleryBonus, spec.chartId, spec.guestCenter)

const scores: number[] = []
for (let i = 0; i < trials; i++) {
    scores.push(new Live(context).simulate().score)
}

process.stdout.write(
    JSON.stringify({
        simulator: 'reference',
        spec: spec.name,
        chart: {
            id: spec.chartId,
            title: chart.title,
            attribute: chart.attribute,
            group: chart.group,
            noteCount: chart.notes.length,
        },

        teamStat: teamStat.map(({ base, param, trick }) => ({ base, param, trick })),
        derived: {
            groupMultipliers: context.groupMultipliers,
            attributeMultipliers: context.attributeMultipliers,
            maxHp: context.maxHp,
            noteTriggers: context.noteTriggers,
            comboTriggers: context.comboTriggers,
            perfectTriggers: context.perfectTriggers,
            starPerfectTriggers: context.starPerfectTriggers,
            sisScoreMultipliers: context.sisScoreMultipliers,
            sisHealMultipliers: context.sisHealMultipliers,
            priorities: context.priorities,
            
            resolvedSkills: context.skillInfos.map(({ card, accessory }) => ({
                card: {
                    triggerType: card.trigger.type,
                    chance: card.trigger.chances[0],
                    threshold: card.trigger.values[0],
                    effectType: card.effect.type,
                    duration: card.effect.durations[0],
                    value: card.effect.values[0],
                },
                accessory: accessory
                    ? {
                          chance: accessory.trigger.chances[0],
                          threshold: accessory.trigger.values[0],
                          effectType: accessory.effect.type,
                          duration: accessory.effect.durations[0],
                          value: accessory.effect.values[0],
                      }
                    : null,
            })),
        },
        trials: scores.length,
        scores,
    })
)
