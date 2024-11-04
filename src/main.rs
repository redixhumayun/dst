use std::{cell::RefCell, collections::VecDeque, error::Error, future::Future};

trait Stream {
    type Item;
}

trait AsyncIO {
    async fn await_future<T>(&self, fut: impl Future<Output = T>) -> T;
    async fn await_stream<T>(&self, stream: impl Stream<Item = T>) -> Option<T>; //  need to add the futures crate here for the stream trait
}

pub trait Kafka {
    async fn produce(&self, topic: &str, payload: &[u8]);
    async fn consume(&self, topic: &str) -> Vec<u8>;
}

trait Redis {
    async fn set(&self, key: &str, value: &str) -> Result<(), Box<dyn Error>>;
    async fn get(&self, key: &str) -> Result<String, Box<dyn Error>>;
    async fn publish(&self, channel: &str, message: &str) -> Result<(), Box<dyn Error>>;
    async fn subscribe(&self, channel: &str) -> future::Stream<Item = String>; //   need to import the futures crate here
}

struct KafkaClient<'a, IO: AsyncIO> {
    io: &'a IO,
    producer: FutureProducer, // some type that implements rdkafka producer
    consumer: StreamConsumer, // some type that implements rdkafka consumer
}

impl<'a, IO: AsyncIO> Kafka for KafkaClient<'a, IO> {
    async fn produce(&self, topic: &str, payload: &[u8]) {
        let record = FutureRecord::to(topic).payload(payload);
        let future = self.producer.send(record);
        self.io.await_future(future)
    }
    async fn consume(&self, topic: &str) -> Vec<u8> {
        self.consumer.subscribe(topic).unwrap();

        let stream = self.consumer.stream();
        let mut messages = Vec::new();
        while let Some(result) = self.io.await_stream(stream).await {
            match result {
                Ok(msg) => {
                    if let Some(payload) = msg.payload() {
                        messages.push(payload);
                    }
                }
                Err(e) => eprintln!("Error consuming messages from Kafka {:?}", e),
            }
        }

        messages
    }
}

struct RedisClient<'a, IO: AsyncIO> {
    io: &'a IO,
    connection: Connection, //  the connected instance to Redis (probably a TCP socket)
}

impl<'a, IO: AsyncIO> Redis for RedisClient<'a, IO> {
    async fn set(&self, key: &str, value: &str) -> redis::RedisResult<()> {
        let set_fut = self.connection.set(key, value);
        self.io.await_future(set_fut).await;
    }

    async fn get(&self, key: &str) -> redis::RedisResult<String> {
        let get_fut = self.connection.get(key);
        self.io.await_future(get_fut).await;
    }

    async fn publish(&self, channel: &str, message: &str) -> redis::RedisResult<()> {
        let pub_fut = self.connection.publish(channel, message);
        self.io.await_future(pub_fut).await.unwrap();
    }

    async fn subscribe(&self, channel: &str) -> impl futures::Stream<Item = String> {
        let mut pubsub = self.connection.as_pubsub();
        pubsub.subscribe(channel).await.unwrap();

        self.io
            .await_stream(pubsub.on_message().map(|msg| msg.get_payload().unwrap()))
    }
}

pub struct IO {
    //  what fields do I need here?
}

impl IO {
    pub fn new() -> Self {
        Self {}
    }
}

impl AsyncIO for IO {
    async fn await_future<T>(&self, fut: impl Future<Output = T>) -> T {
        todo!()
        //  the actual implementation goes here
    }
    async fn await_stream<T>(&self, stream: impl Stream<Item = T>) -> Option<T> {
        todo!()
        //  the actual implementation goes here
    }
}

pub struct SimulatedIO {
    //  what do I need here?
    inner: Box<dyn AsyncIO>,
    rng: RefCell<ChaCha8Rng>,
}

impl SimulatedIO {
    fn new(seed: u64) -> Self {
        let inner = IO::new();
        let rng = ChaCha8Rng::seed_from_u64(seed);
        Self { inner, rng }
    }
}

impl AsyncIO for SimulatedIO {
    async fn await_future<T>(&self, fut: impl Future<Output = T>) -> T {
        todo!()
        //  return simulated responses here
    }
    async fn await_stream<T>(&self, stream: impl Stream<Item = T>) -> Option<T> {
        todo!()
        //  return simulated responses here
    }
}

struct FaultInjector {
    rng: ChaCha8Rng,
    connection_failure_prob: f64,
    packet_loss_prob: f64,
    out_of_order_prob: f64,
    intermittent_disconnect_prob: f64,
}

impl FaultInjector {
    pub fn new(
        rng: ChaCha8Rng,
        connection_failure_prob: f64,
        packet_loss_prob: f64,
        out_of_order_prob: f64,
        intermittent_disconnect_prob: f64,
    ) -> Self {
        Self {
            rng,
            connection_failure_prob,
            packet_loss_prob,
            out_of_order_prob,
            intermittent_disconnect_prob,
        }
    }

    pub fn should_inject_connection_failure(&mut self) -> bool {
        self.rng.gen_bool(self.connection_failure_prob)
    }

    pub fn should_inject_packet_loss(&mut self) -> bool {
        self.rng_gen_bool(self.packet_loss_prob)
    }

    pub fn should_inject_out_of_order(&mut self) -> bool {
        self.rng_gen_bool(self.out_of_order_prob)
    }

    pub fn should_inject_intermittent_disconnect(&mut self) -> bool {
        self.rng_gen_bool(self.intermittent_disconnect_prob)
    }
}

struct SimulatedKafka<'a, IO: AsyncIO> {
    inner: KafkaClient<'a, IO>,
    fault_injector: FaultInjector,
    incoming_buffer: VecDeque<Vec<u8>>,
}

impl Kafka for SimulatedKafka<'_, IO> {
    async fn produce(&self, topic: &str, payload: &[u8]) {
        if self.fault_injector.should_inject_packet_loss() {
            println!("Kafka fault injected: packet loss on message to topic: {topic}");
            return ();
        }

        if self.fault_injector.should_inject_intermittent_disconnect() {
            println!("Kafka fault injected: disconnected from downstream Kafka");
            //  should return an error here indicating that downstream kafka was disconnected
            return ();
        }

        self.inner.produce(topic, payload);
        todo!()
    }

    async fn consume(&self, topic: &str) -> Vec<u8> {
        let message = self.inner.consume(topic).await;
        if self.fault_injector.should_inject_out_of_order() {
            self.incoming_buffer.push_back(message);
            if !self.incoming_buffer.is_empty() {
                let delayed_message = self.incoming_buffer.pop_front().unwrap();
                println!(
                    "Delivering delayed (out-of-order) message: {}",
                    delayed_message
                );
                return Some(delayed_message);
            }
            None
        } else {
            if !self.incoming_buffer.is_empty() {
                Some(self.incoming_buffer.pop_front().unwrap())
            } else {
                Some(message)
            }
        }
        todo!()
    }
}

struct SimulatedRedis<'a, IO: AsyncIO> {
    inner: RedisClient<'a, IO>,
    fault_injector: FaultInjector,
}

impl Redis for SimulatedRedis<'_, IO> {
    async fn set(&self, key: &str, value: &str) -> Result<(), Box<dyn Error>> {
        if self.fault_injector.should_inject_connection_failure() {
            println!("Redis fault injected: connection failure");
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "Connection refused",
            )));
        }

        self.inner.set(key, value).await;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<String, Box<dyn Error>> {
        if self.fault_injector.should_inject_connection_failure() {
            println!("Redis fault injected: connection failure");
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "Connection refused",
            )));
        }

        self.inner.get(key).await;
        Ok(())
    }

    async fn publish(&self, channel: &str, message: &str) -> Result<(), Box<dyn Error>> {
        if self.fault_injector.should_inject_connection_failure() {
            println!("Redis fault injected: connection failure");
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "Connection refused",
            )));
        }

        self.inner.publish(channel, message).await;
        Ok(())
    }

    async fn subscribe(&self, channel: &str) -> future::Stream<Item = String> {
        if self.fault_injector.should_inject_connection_failure() {
            println!("Redis fault injected: connection failure");
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "Connection refused",
            )));
        }

        self.inner.subscribe(channel).await;
        Ok(())
    }
}

/// How do I partition the machine's cores such that I can run a separate event loop per core(separate Tokio or Glommio runtime) and
/// run each loop deterministically
/// The simplest way would be for me to hash the customer id % by the number of cores so each customer id is assigned to a specific core
/// but this doesn't account for the fact that incoming traffic is very skewed. Need to think of a better partitioning strategy.
fn main() {
    println!("Hello, world!");

    //  When running the application, the entire thing needs to run in a loop, consuming messages from Kakfa, reading config from Redis
    //  processing the required data and publishing to a downstream Kafka
    //  When running the simulation, the entire loop will run with the simulated mocks, and the entire thing should be reproducible from the seed

    let seed = 1234; //  generate a random seed here
    let connection_failure_prob = 0.1; //  10% chance of connection failure
    let packet_loss_prob = 0.15; //  15% chance for packet loss
    let out_of_order_prob = 0.1; // 10% chance for out-of-order delivery
    let intermittent_disconnect_prob = 0.2; // 20% chance for intermittent disconnect
    let rng = ChaCha8Rng::seed_from_u64(seed);

    let fault_injector = FaultInjector::new(
        rng,
        connection_failure_prob,
        packet_loss_prob,
        out_of_order_prob,
        intermittent_disconnect_prob,
    );
    let simulated_io = SimulatedIO::new(seed);
    //  create a kafka & redis client
    //  create a simulated kafka and redis client here and feed in the actual clients as dependencies
    //  run the application logic in a loop
}
