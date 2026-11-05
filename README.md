# Takaramonozu
High-Performance Simulator and Team Building (Eventual) Optimizer for Love Live! School Idol Festival

# Why

This is an obvious question. At the time of writing this README, SIF has been dead for more than 3 years. On top of that, the community already had existing tools that they used, such as Burrito's team simulator^1, which was widely used by many to approximate scores in the latter half of SIF's life, as well as various team-building tools, such as LLATB^2 and Pork Bun's (which I can't link to because it was closed source and the site is no longer up.)

"Solving" LLSIF has been an interest of mine since 2014 and has greatly influenced my trajectory in life, particularly in my education, to develop the skillset needed to come up with the best solution possible. Back in 2016, I had created a Google sheet^3 that would calculate the expected score of a given song with given cards, skill levels, SIS, guests, accuracy, etc. For the time it was a useful tool, as no team builders or simulators were out yet (to my knowledge). While I had fallen off of SIF during its twilight years, never completing a full solution has never sat well with me - for years now I've continually wondered how each new skill I've developed could be applied to this long-standing problem.

Existing tools have their fair share of gaps. One is mechanical. Burrito's sim, while fantastic, doesn't quite implement *all* of SIF, leaving a few mechanics off the table, and, as I learned while working on this, also had a few implementation errors. (See DIVERGENCES.md for a full breakdown on the differences between the two simulators.) Another is speed. I believe that SIF is far, FAR too complex of a game to come up with a closed-form or even a heuristic solver for. Which is why I opted for two exceptionally compute-hungry methods to solve in my implementation - genetic algorithms and Reinforcement Learning. And in order for those methods to converge on a solution in any reasonable amount of time, a blisteringly fast monte carlo simulator is required. Plus, honestly, I just like writing simulator code. It's fun to see the funny number go up. or go down. I guess both because there's actually a lot of funny numbers you monitor like IC or throughput or wall-clock.

# Roadmap

- The Data preprocessor, deck manager, team compiler, and monte carlo simulator are all feature complete!

- (In-progress) Profile everything. Make some sample teams to run through pgo. See what coz has to say. Check DHAT on the deck layer. Check Valgrind to see how our cache usage works. Hook the sample teams up with rayon and see how we scale with cores. and I guess some boring perf / samply to get wc time down (just kidding regular ol' perf is probably the most important one).
- (In-progress) Bring in Burrito sim. Two aspects to this and I'm not sure what I'm going to do first. Like wholesale. There's two directions here.
    - (In-progress) The backend of burrito to benchmark against. It shouldn't be *too* much work to get a functional harness so that the same team can be fed into and evaluated through both sims. Then simply let it rip and get some numbers. I guess this is needed for validation too but tbh not everything will be apples to apples. See DIVERGENCES.md
    - The frontend. I am not a frontend dev. I plan to vendor the frontend from burrito's sim wholesale, and see what needs to be done to wire it up to this simulator's backend. While I have some frontend / ts experience, it's not something that's particularly engaging to me and find it, as the kids say, cringe. But it must be done.
- The optimizer. This will be fun and some details are still TBD. Probably going to have one or more metaheuristics. I'm thinking WASM compatibility so a team can be optimized in a browser and there would be like a drop down where you select which metaheuristic you want. I'm probably going to hand-roll a genetic algorithm here (I love genetic algorithms by the way they're so cool). The simulator itself is embarrassingly parallel so I'll probably go with an island model - each thread does their own thang and every now and then they... well I need to figure out how the migrations will work I have like 5 ideas in my head not sure what will be best. Will need a fidelity ladder as well as successive halving. But really the goal with the optimizer is just to have instant inference for any potential set of cards/accs/sis a user might have. I'm considering a few approaches to this, but in the "where" and the "how". for the "where", the mind is telling me to FFI to Python. All of the DL and RL frameworks are there. They're mature. You have experience doing that in Python. Just take the plunge into Pyo3. My heart is telling me "you hate Python and you want to see the Rust RL ecosystem mature. Make something, make it good. and open source it so maybe others might consider using Rust for this somewhere down the line too." Not sure if my mind or my heart will win. As for the "how", there's a few interesting angles / papers I've been reading that I'd like to give a chance. Idea one is to treat it as a contextual bandits problem and throw REINFORCE at it. The other is NCO with an action masked PPO. tbd, subject to change. We'll see what's good. I genuinely think this sim is fast enough to pull this off though like this thing is FAST 2 million trials per second across 24 threads on my consumer CPU with SMT imagine if I rented a crazy super computer and let it rip for a few hours.

# Current Structure

This will be a super brief overview.

Raw data -> Preprocessor -> Deck -> Simulator -> Bench

Eventually the Frontend and Optimizer crates will write to Deck and consume Simulator but they're not ready yet.

Raw data from various sources (kirara, schoolido.lu, burrito's sim TODO: add links / references) from data is processed in the `preprocess` crate. It turns it from .json into .rkyv, does some transformations where necessary, and otherwise packs the data as the simulator will consume it. This contains pretty much every basic data structure for the game. What a song is, what a Leader Skill is. What a Gallery is. Stuff of that nature.

Then the `Deck` layer constructs objects from the processed data. This is where users would add cards, accessories, and SIS to their collection, defining level, skill level, idolization, etc. There's commands to add, delete, and modify items from a collection. This is also the layer where `Team` construction happens, so any validation of whether something can be equipped on a `Team` happens here, as well as the initial stat calculations. `Deck` `compile`s a `Team` that is then passed to `Simulator`. This also isn't an API that's meant to be called directly - the frontend and optimizer are meant to consume it which is why I'm not listing down what commands one could use to construct their own `Team`.

One note I'll make here is that I think I made a bit of an oopsie by using rkyv for cards, accessories, and sis since deck consumes them and produces mutable .json files for a user's contents. But uh, at least they can load them super fast before they get written to json?? Songs are the ones that really make sense to have as rkyv + memmap'd.

Then the `Simulator` layer is in charge of running through an actual simulation of the game. It starts by looking at the team and pre-computing as much as possible, deciding which of the 2096 copies of the gameplay loop function called `run` that it should use based on the skills present on the team, setting up and rolling common random numbers, and otherwise handling all gameplay mechanics to arrive at a final score. This is also a lib that's not meant to be called directly. The benchmark suite, optimizer, and frontend do/will consume this.

That was not as brief as I wanted.

# Benchmark Results

While this is being compared to another simulator, I want to make it clear that this repo is a complete rewrite from scratch as an entirely bespoke implementation. This was not "Hey Claude, take this typescript simulator and translate it to Rust. Also implement the things they didn't. Make no mistakes." Rather, Burrito's simulator served as both something for me to check when I was unsure on a specific mechanic, as well as being something to compare against to validate my results, which I attribute to why DIVERGENCES.md has as many entries as it does.

Full suite:

```
cargo bench -p bench
```

Can pass in either `tiers` or `trial`

```
cargo bench -p bench [tiers | trial]
```

`trial` is for measuring the full playthrough of a specific song with a specific team 50,000 times. It builds and compiles the team once and then reuses it for the 50k loops.

`tiers` is for measuring each stage of a simulation, including the setup at the beginning.

---

A custom headless harness was written to run Burrito's code without needing to render any of the frontend. See `bench/ref`.

Benchmarking code that uses all cores/threads is particularly awful on a personal computer because I don't even think it's possible to get the machine *truly* idle. I do my best to reasonably close as much as possible before every run and Divan is configured to take a representative sample.

All results were recorded on an AMD Ryzen 7900X. This cpu has 12 cores, 24 threads with SMT.

These are the current results using a variety of teams. The goal was to test each mechanic individually so there's *mostly* one mechanic per benchmark instance.

| spec | one thread | all threads | burrito |
| --- | ---: | ---: | ---: |
| accessory-skills | 343.7 K/s | 5.323 M/s | 5.882 K/s |
| amplify-encore-scorers | 278.3 K/s | 4.495 M/s | 7.485 K/s |
| combo-fevers | 147.1 K/s | 2.347 M/s | 3.796 K/s |
| everything | 46.9 K/s | 728 K/s | x |
| healers-heal-sis | 299.5 K/s | 4.558 M/s | 5.533 K/s |
| n-cards-only | 521.7 K/s | 6.452 M/s | x |
| nine-psus | 141.1 K/s | 2.146 M/s | 3.626 K/s |
| param-boost-sync | 156.4 K/s | 2.424 M/s | x |
| parity-scorers | 457.1 K/s | 6.206 M/s | 8.317 K/s |
| perfect-locks-trick-sis | 161 K/s | 2.6 M/s | x |
| score-scorers | 211 K/s | 2.949 M/s | x |
| scorer-sru | 368.1 K/s | 4.918 M/s | 6.457 K/s |
| scorers | 361.5 K/s | 4.982 M/s | 4.944 K/s |
| spark-encore | 319.5 K/s | 4.755 M/s | 6.879 K/s |
| star-notes | 272.9 K/s | 3.817 M/s | 4.897 K/s |
| swing-scorers | 391.9 K/s | 5.461 M/s | x |
| time-triggers | 339.2 K/s | 4.295 M/s | 6.857 K/s |
| total-tricks | 181.6 K/s | 2.813 M/s | x |
| two-srus | 238.7 K/s | 3.395 M/s | x |
| year-group-chains | 280.9 K/s | 4.025 M/s | x |

x means Burrito's sim can't play that team:

# Score Comparison

The below table validates that the scores that both simulators generate matches for any mechanics where they're not expected to diverge. See `bench/compare.R` for more detail on why everything here was chosen but the tldr is that normality should not be assumed for the result's distributions because some mechanics (looking at you, coinflip) can lead to non-normal results.

So the results of this table are focused around both center and shape. mean, gap and z all relate to measuring the centers and KS D, KS p, p1/p50/p99 all measure the shape.

I won't be going into the mechanics of the Kolmogarov Smirnov test here. D measures the difference between both CDFs, p is the how likely that difference is to occur, p1/p50/p99 are the gaps between the respective percentiles to ensure tail behavior matched between simulators.

| spec | Takaramonozu mean | burrito mean | gap | z | KS D | KS p | p1 / p50 / p99 gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| accessory-skills | 3,654,344 | 3,651,727 | +0.072% | +1.6 | 0.0145 | 0.031 | +0.06% / +0.16% / +0.13% |
| amplify-encore-scorers | 1,963,182 | 1,964,489 | -0.067% | -0.3 | 0.0074 | 0.652 | -2.42% / +0.14% / -0.69% |
| combo-fevers | 3,185,606 | 3,186,155 | -0.017% | -0.5 | 0.0095 | 0.334 | -0.04% / -0.01% / -0.07% |
| healers-heal-sis | 2,895,379 | 2,895,207 | +0.006% | +0.1 | 0.0063 | 0.830 | -0.47% / -0.01% / +0.42% |
| nine-psus | 2,488,747 | 2,488,842 | -0.004% | -0.1 | 0.0061 | 0.851 | +0.07% / -0.00% / -0.08% |
| parity-scorers | 1,131,722 | 1,131,747 | -0.002% | -0.0 | 0.0068 | 0.744 | -0.02% / -0.09% / +0.08% |
| scorer-sru | 1,805,315 | 1,807,240 | -0.107% | -1.2 | 0.0086 | 0.443 | -0.35% / -0.03% / +0.24% |
| scorers | 2,786,300 | 2,785,580 | +0.026% | +0.4 | 0.0066 | 0.776 | -0.06% / +0.03% / +0.21% |
| spark-encore | 882,597 | 884,781 | -0.247% | -1.2 | 0.0093 | 0.359 | +1.07% / -0.25% / -0.03% |
| star-notes | 1,779,549 | 1,778,969 | +0.033% | +0.4 | 0.0073 | 0.669 | -0.36% / +0.03% / -0.05% |
| time-triggers | 1,099,715 | 1,101,067 | -0.123% | -1.5 | 0.0113 | 0.155 | +0.06% / -0.13% / -0.23% |

In conclusion, throw in a Bonferroni correction for that one accessory-skills p-value and nothing here looks out of place and I feel confident saying the scores agree where they're supposed to agree. For the record, not listed here, but they also disagree where they're supposed to disagree and pretty much anything from DIVERGENCES.md on any of the specs would result in a tangible gap.

# References

I'll wire these up to the right spots of the readme with links later.

https://github.com/NonSpicyBurrito/sif-team-simulator

https://github.com/Joshua1989/LLSIF-AutoTeamBuilder

https://docs.google.com/spreadsheets/d/1ecVUlSYMU5oBAvSbhgmbUjPlJZTklf2OF6_s3MY4dTM/edit?gid=517169788#gid=517169788
