This document outlines any modeling differences between this repo's simulator vs Burrito's.

Some of these are entire features / skills that are on a ton of cards. Others are very minuscule differences that practically will never come up.

# Mechanics

- **Missing Sync, Score-Scorers, Year-Group, and Total Trick**: Burrito's simulator does not model the `Sync` skill effect, the `Score` trigger condition, the Year Group (aka "Chain Scorer") trigger condition, or the `Total Trick` skill effect. This simulator implements all four in full.
- **N cards**: This sim includes all N cards but Burrito's doesn't. I didn't see a reason not to include every usable card even if practically no one will ever put them on a team past the first 30 minutes of playing. (This one's not really a "mechanic" but it didn't fit under anything else.)

# General Scoring

- **Intermediate Rounding of Team Strength** - Burrito's sim keeps all stats as floats throughout the stat pipeline, only rounding for display at the end. This sim floors at each step, like the game. This doesn't matter in 99% of cases but will occasionally lead to team strength being slightly off on burrito's side.
- **Note Flooring**: This simulator floors the score to the closest integer on each note. Burrito's keeps it as a float. Small from note-to-note, but adds up to Burrito slightly over-estimating over the course of an entire song.
- **Charm Flooring**: Basically the same thing as above but doesn't impact literally every single note. If a a scorer has a Charm equipped and the scorer's skill value is an odd number, Burrito does not floor `value * 2.5`, whereas Takaramonozu does.
- **Promo Level-Cap Bonus**: Burrito only includes the level cap bonuses for levels 101-500 for regular URs. This repo includes promo URs as well. I understand their decision. Why would anyone serious about their score ever give level up gems to a promo UR!?
- **Swing-Holds**: Both simulators used the same `songs.json`. During my testing, I went looking for a note in `songs.json` that was simultaneously a hold+swing at the same time. To my surprise, none showed up, which was weird because I distinctly remember a number of charts having hold notes at the end of a chain of swings. This matters because swing+holds have their own tap multiplier. Turns out the source data was just wrong in a number of places. This repo uses the corrected `songs.json` with properly-tagged swing-holds.
- **Simultaneous Everything**: Burrito's keeps time as f64, which leads to floats occasionally having simultaneous note spawn or note taps happen in an unpredictible order. This repo uses integers for time making these resolutions deterministic.
- **Seeding and RNG**: Burrito's RNG comes from `Math.random()` calls. There's no seeding, so a run on the same song with the same team with the same config will lead to different results. This sim is entirely reproducible. No common random numbers or different PRNG streams for different sources on Burrito's side either, while there's different streams for Judgements, Skills, Sync, and Coin flip here (though tbh the coin flip one is overkill and unnecessary).

# Skills

- **Param Boost Param Calculation**: Burrito leaves leader and guest skills out of the param boost calculation. This sim takes the card's final stat *after* adding team bonuses and leader skills, *then* applies param boost.
- **Param Boost Application**: Targeting does not work correctly for param boost in Burrito's sim - it applies it to all 9 lanes no matter what. This sim applies it to the specific cards that are targeted.
- **Param Rounding**: This applies to the boosts granted by Param Boost and the Trick SIS. Burrito keeps it as a float, this sim ceilings it.
- **Combo Fever rounding**: We floor the products of the bonus * combo multipler. Burrito rounds it. This one is super minor, only ever 1 point here or there in practice.
- **Overheal Ceiling**: We clamp to 200%. Burrito allows going slightly past 200%. Looks like they have `1 + hearts * heartBonus` which can go up to 204%.
- **Hearts below 9**: Technically we're less accurate here because ours is impossible and theirs matches the game. The least amount of HP you can have in a song in the real game is 9. This sim allows running the simulator with empty lanes, so 1 is the minimum HP. It simply treats any team with less than 9 HP as if it had 9 HP. This was done for optimizer purposes to score partial teams (although the more I think about the optimization, I probably won't even model it like that but oh well glad I gave myself the flexibility).
- **SRU on SRU**: Burrito allows an SRU to boost another SRU's roll. This simulator doesn't.
- **Simultaneous Timer Triggers**: There's two things going on here that end up having ramifications (that in practice would never come up because who uses Timer skills) A) Burrito processes each timer skill individually and B) Burrito rolls Timer activations left-to-right (whereas everything else in the game is right->left). On its own this is harmless, but this can subtly change who takes an amp boost, when accessory effects can apply to other lanes (usually a card needs to wait until the next "moment" before it receives a skill that activates), and allows year-group triggers to check on each skill activation, whereas one "moment" of activations should only ever allow for one contribution to the chain. This sim resolves them all at once, from right-to-left.
- **Spark Charges**: Two issues here. Burrito gives a charge when *any* accessory activates on an Encore card. Likewise, Burrito does *not* give a charge when an Encore accessory activates on any card. This is the opposite of how the game handles it, which is fixed in this sim. Encore accs grant a spark charge regardless of what they're on and non-Encore accs can't grant a charge if they're on Encore cards.
- **Spark Activation**: If a Spark does not have enough charges to activate, it should not even attempt to roll its skill. Burrito's sim allows the skill to attempt to roll, which can potentially eat an amp charge. This sim blocks the spark from rolling unless the threshold is met.

# SIS

- **Accessory Equip**: Most accessories are tied to a single card_id, unless the acc is skill level 8, at which point it can be used on any card of that character. This simulator enforces that restriction. Burrito allows a character-locked accessory to be equipped on different card_ids at any skill level.
- **Nonet**: Burrito's sim applies the Nonet bonus whether the character-set condition is met or not. The onus is on the user to equip a Nonet appropriately. This sim allows a Nonet to be equipped but will not grant the bonus if its 9-unique-characters-from-the-same-group condition is not met.
- **Trick SIS**: Similar to Nonet. The Trick SIS A) Can only be equipped on a card of the same attribute and B) Does nothing if the card/sis attribute do not match the song's attribute. Taken together, that means a Cool card can never activate a Trick SIS it has equipped while playing a Smile song. Burrito's sim allows equipping the SIS and will give the bonus.
- **Charm and Heal SIS**: These should be locked to scorer/healer cards, which are mutually exclusive. Burrito does not check for legality for equipping SIS, which allows for this combination to be equipped on one card.
- **Off-Attribute stat SIS**: Due to the way SIS are set up in Burrito's sim, the SIS are always assumed to match the song's attribute. This sim treats each unique SIS as its own thing. While in 99.99% of cases you're going to match SIS to attribute, there's some edge cases where you may throw an off-attribute kiss in a slot where you have a center / guest that boosts that off-attribute which can lead to minor team strength differences.

# In Burrito, but not here
- **Judgements**: This is one where Burrito does more than this sim. We only model Perfects and Greats and assume a full combo. Burrito allows for a fully customizable Perfect/Great/Good/Bad/Miss ratio and combo-break logic. A big use for Burrito's sim was determining the success rate of an "AFK Team" (a team that misses every single note) passing a song, which is completely out of scope for this repo.
- **Overheal**: Since this simulator does not model survival, and the only way to lose HP is from good/bad/misses, it can make the assumption that the user is always at full health. This matters for earning overheal hearts, as they're only obtained at full health. Burrito properly tracks HP before granting overheal.
- **Med fes / chal fes / LA bonuses**: There were event modes that could give bonuses to skill activation rate, tap score, or reduce skill activation rate. This sim does not attempt to model the special effects from those modes.
