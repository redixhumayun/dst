## SimulatIOn
SimulatIOn is a Determinstic Simulation Testing(DST) setup for educational purposes. If you're interested in learning about more about DST, read [this post](https://notes.eatonphil.com/2024-08-20-deterministic-simulation-testing.html).

### Running This Project
* Clone the repo or fork the repo
* Build it with `cargo build`
* Run the simulator with `cargo run -- --simulate`. If you want to pass a specific seed value, `SEED=12920692343208412637 cargo run -- --simulate`
* Run the visualisation engine with `cargo run -- --game`. If you want to pass a specific seed value, `SEED=12920692343208412637 cargo run -- --game`

### Modeling Errors
This project models a few standard errors:
* Connection errors
* File write failures
* Corrupted messages via Kafka

The base idea is that with a specific seed, you can recreate a completely deterministic run.

## Resources

1. https://github.com/penberg/hiisi
2. https://github.com/penberg/limbo
3. https://notes.eatonphil.com/2024-08-20-deterministic-simulation-testing.html
4. https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vopr.zig
