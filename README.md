## What Am I Trying To Show
* A simple loop of reading from upstream Kafka, reading some data from Redis and optionally writing to a downstream Kafka. The important thing here is to hide everything behind an IO interface
* The IO interface should be swappable with something that runs simulated IO
* The loops should be thread-per-core so run as many threads as there are cores, each with it's own loop. (is this really important?)
* Perhaps a TUI based game that allows users to "see" their simulation and they can keep fixing their code and re-run the simulation to pass the program


## Implementation Notes
* Okay, I have some basic code that can read from Kafka, Redis & then write to disk. I now want to be able to simulate these forms of IO.
* After simulating IO, I should be able to inject faults into the operations
* What are my main operations?
  * connecting to Kafka
  * reading from Kafka (can data arrive out-of-order from Kafka?)
  * reading config from Redis
  * writing to a file
  * reading from a file (?)


TODO: Need to think about what kind of faults can be injected here. 

## Resources

1. https://github.com/penberg/hiisi
2. https://github.com/penberg/limbo
3. https://notes.eatonphil.com/2024-08-20-deterministic-simulation-testing.html
4. https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vopr.zig
