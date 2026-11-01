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

This is still a WIP, especially since perfmaxxing is still actively happening little-by-little. I don't know if I'll remember to update the numbers every single time I make a change.

Benchmarking code that uses all cores/threads is particularly awful on a personal computer because I don't even think it's possible to get the machine *truly* idle. I do my best to reasonably close as much as possible before every run and Divan is configured to take a representative sample.

All results were recorded on an AMD Ryzen 7900X. This cpu has 12 cores, 24 threads with SMT.

These are the current results using a variety of teams. The goal was to test each mechanic individually so there's *mostly* one mechanic per benchmark instance.

I'll add the reference result comparisons when they're ready.
I also plan to add a score distribution comparison to ensure that both sims agree where they're supposed to agree and disagree sensibly where they're not expected to agree.

| spec | one thread | all threads | burrito |
| --- | ---: | ---: | ---: |
| accessory-skills | 293.7 K/s | 3.814 M/s | |
| amplify-encore-scorers | 267 K/s | 3.568 M/s | |
| combo-fevers | 85.99 K/s | 1.244 M/s | |
| everything | 37.83 K/s | 605 K/s | |
| healers-heal-sis | 253.8 K/s | 3.528 M/s | |
| n-cards-only | 400 K/s | 4.72 M/s | |
| nine-psus | 90.18 K/s | 1.43 M/s | |
| param-boost-sync | 121.9 K/s | 1.825 M/s | |
| parity-scorers | 374.8 K/s | 4.888 M/s | |
| perfect-locks-trick-sis | 111.5 K/s | 1.778 M/s | |
| score-scorers | 168.1 K/s | 2.212 M/s | |
| scorer-sru | 182.6 K/s | 2.328 M/s | |
| scorers | 351.3 K/s | 3.774 M/s | |
| spark-encore | 264.7 K/s | 3.431 M/s | |
| star-notes | 197 K/s | 2.514 M/s | |
| time-triggers | 270.2 K/s | 3.014 M/s | |
| total-tricks | 144.5 K/s | 2.13 M/s | |
| year-group-chains | 210.9 K/s | 2.617 M/s | |

# References

I'll wire these up to the right spots of the readme with links later.

https://github.com/NonSpicyBurrito/sif-team-simulator

https://github.com/Joshua1989/LLSIF-AutoTeamBuilder

https://docs.google.com/spreadsheets/d/1ecVUlSYMU5oBAvSbhgmbUjPlJZTklf2OF6_s3MY4dTM/edit?gid=517169788#gid=517169788
