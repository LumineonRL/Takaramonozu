/// <reference path="./globals.d.ts" />


// Headless driver for burrito's sim so teams can be made from the command-line instead of
// in the frontend.
// Reads a spec from `bench/specs`, calls the Context/Live parts of the simulator and returns the result.
// No game logic changes here at all. It literally just strips out any frontend components just to run
// the simulator and nothing else. And adds seeded RNG to make results more consistent without affecting performance.
//
// `cd bench/reference && npm ci && npm run build`
// `node bench/reference/dist/runner.mjs <spec.json> <trials> [seed]`

import { existsSync, readFileSync } from 'node:fs'
import { basename, dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { accessories, cards, charts, initDatabase, sises } from './sif-team-simulator/src/database'
import { Difficulty } from './sif-team-simulator/src/database/Chart'
import { Context } from './sif-team-simulator/src/core/simulation/Context'
import { Live } from './sif-team-simulator/src/core/simulation/Live'
import { calculateTeamStat } from './sif-team-simulator/src/core/stats'
import { Team } from './sif-team-simulator/src/core/Team'
import { CenterSkill } from './sif-team-simulator/src/database/Center'
import { EffectType } from './sif-team-simulator/src/database/Skill'

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
const DATA = join(SIM_ROOT, '../../../data')

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

// Everything (most) what's below here is for parsing the specs from
// bench/specs/*.json.
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
}

type Lane = {
    card: number
    idolized?: boolean
    skill_level?: number
    cap_level?: number
    sis_slots?: number | null
    accessory?: { id: number; skill_level: number } | null
    sis?: number[]
}

type BenchSpec = {
    run_burrito: boolean
    chart: { title: string; difficulty: keyof typeof Difficulty }
    lanes: Lane[]
    guest?: number | null
    accuracy: number
    speed: number
}

type OurChart = { title: string; difficulty: string }
type OurSis = { id: number; effect: string; value: number }

const RATE_UNIT = 10_000
const SIS_TYPES: Record<string, string> = {
    'self percent boost': 'self',
    'all percent boost': 'team',
    'self flat boost': 'flat',
    charm: 'score',
    heal: 'heal',
    trick: 'plock',
}

function readJson<T>(path: string): T {
    return JSON.parse(readFileSync(path, 'utf8')) as T
}

const [specPath, trialsArg, seedArg] = process.argv.slice(2)
if (!specPath || !trialsArg) throw new Error('Should be: runner <spec.json> <trials> [seed]')
const name = basename(specPath, '.json')
const bench = readJson<BenchSpec>(specPath)
const trials = Number(trialsArg)
if (!Number.isInteger(trials) || trials < 1) throw new Error(`bad trial count ${trialsArg}`)
if (!bench.run_burrito) throw new Error(`${name}: run_burrito is false`)
if (bench.lanes.length !== 9) throw new Error(`${name}: ${bench.lanes.length} lanes`)

// Charts are mostly the same between the two simulators (duh, I used the data from that sim as a base)
// The big difference is that they key on Live_.*.json and this sim does it on (title, difficulty)
// (with some special renaming done for songs that have multiple versions of the same song on the same difficulty).
// So the two are rectified by looking up our (title, difficulty), grabbing its key, and then searching burrito with that key.
const ourCharts = readJson<Record<string, OurChart>>(join(DATA, 'songs.json'))
const { title, difficulty } = bench.chart
const keys = Object.keys(ourCharts).filter(
    (key) => ourCharts[key].title === title && ourCharts[key].difficulty === difficulty
)
if (keys.length !== 1) throw new Error(`${name}: ${keys.length} charts of ${title} ${difficulty}`)
const chartId = keys[0]

await initDatabase(Difficulty[difficulty])

const chart = charts.get(chartId)
if (!chart) throw new Error(`${name}: burrito has no ${chartId}`)

const ourSis = new Map(readJson<OurSis[]>(join(DATA, 'sis.json')).map((sis) => [sis.id, sis]))

// There's a million ways things could potentially fail that burrito doesn't support but this
// sim does and what's below here is there error handling for that.
// For example, N cards are missing from burrito, so it'd throw an error if a card id of an N was
// passed in. Same with missing accs (but there shouldn't be any of those), any card that's a score-scorer
// or sync.
const guest = bench.guest ?? undefined
const missing: string[] = []
bench.lanes.forEach((lane, slot) => {
    const where = `lane ${slot + 1}`
    if (!cards.has(lane.card)) missing.push(`${where}: burrito has no card ${lane.card}`)
    if (lane.accessory && !accessories.has(String(lane.accessory.id))) {
        missing.push(`${where}: burrito has no accessory ${lane.accessory.id}`)
    }
    for (const id of lane.sis ?? []) if (!ourSis.has(id)) missing.push(`${where}: no SIS ${id}`)
})
if (guest !== undefined && !cards.has(guest)) missing.push(`guest: burrito has no card ${guest}`)
if (missing.length) throw new Error(`${name} can't run on burrito:\n  ${missing.join('\n  ')}`)

const guestCenter = guest === undefined ? null : cards.get(guest)!.center

function member(lane: Lane): SpecMember {
    return {
        cardId: lane.card,
        level: lane.cap_level ?? 100,
        skillLevel: lane.skill_level ?? 1,
        sis: (lane.sis ?? []).map((id) => {
            const row = ourSis.get(id)!
            const type = SIS_TYPES[row.effect]
            // I don't know why I decided to store charms at 1.5x instead of 2.5x.
            // This adjustment is for charms to give them both the 2.5x multiplier.
            return { id, type, value: type === 'score' ? 1 + row.value : row.value }
        }),
        accessory: lane.accessory ? { id: String(lane.accessory.id), level: lane.accessory.skill_level } : undefined,
    }
}

const spec: Spec = {
    name,
    chartId,
    difficulty: Difficulty[difficulty],
    mode: 'normal',
    members: bench.lanes.map(member),
    guestCenter,
    memoryGalleryBonus: [0, 0, 0],
    noteSpeed: bench.speed,
    tapScoreBonus: 0,
    skillChanceBonus: 0,
    skillChanceReduction: 0,
    accuracy: bench.accuracy / RATE_UNIT,
}

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
if (seedArg !== undefined) {
    const next = xoshiro128StarStar(BigInt(seedArg))
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

// For score-scorers, year group triggers, and sync.
let context: Context
try {
    context = new Context(
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
} catch (error) {
    throw typeof error === 'string' ? new Error(`${name} can't run on burrito: ${error}`) : error
}
const syncs = context.skillInfos.flatMap(({ card, accessory }, slot) =>
    card.effect.type === EffectType.Sync || accessory?.effect.type === EffectType.Sync ? [slot + 1] : []
)
if (syncs.length) throw new Error(`${name} can't run on burrito: no Sync, on lanes ${syncs.join(', ')}`)

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
