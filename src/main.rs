use std::sync::{Arc, Mutex};
use std::{collections::HashMap, path::Path, time::Duration};

use async_trait::async_trait;
use clap::Parser;
use futures::stream::StreamExt;
use rand::Rng;
use rand::{seq::SliceRandom, RngCore};
use rand_chacha::{rand_core::SeedableRng, ChaCha8Rng};
use rdkafka::{
    consumer::{stream_consumer::StreamConsumer, Consumer},
    ClientConfig, Message, TopicPartitionList,
};
use redis::AsyncCommands;
use tokio::io::AsyncWriteExt;
use tracing::{error, info, trace, warn};
use tracing_subscriber;

enum Errors {
    KafkaConnectionError,
    NoKafkaMessage,
    RedisConnectionError,
    RedisKeyRetrievalError,
    FileOpenError,
    FileWriteError,
}

impl std::fmt::Debug for Errors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Errors::KafkaConnectionError => write!(f, "Kafka connection error"),
            Errors::NoKafkaMessage => write!(f, "No Kafka message"),
            Errors::RedisConnectionError => write!(f, "Redis connection error"),
            Errors::RedisKeyRetrievalError => write!(f, "Error retrieving redis key"),
            Errors::FileOpenError => write!(f, "Failed to open file"),
            Errors::FileWriteError => write!(f, "Failed to write to file"),
        }
    }
}

impl std::fmt::Display for Errors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Errors::KafkaConnectionError => write!(f, "Kafka connection error"),
            Errors::NoKafkaMessage => write!(f, "No Kafka message"),
            Errors::RedisConnectionError => write!(f, "Redis connection error"),
            Errors::RedisKeyRetrievalError => write!(f, "Error retrieving redis key"),
            Errors::FileOpenError => write!(f, "Failed to open file"),
            Errors::FileWriteError => write!(f, "Failed to write to file"),
        }
    }
}

impl std::error::Error for Errors {}

#[derive(Eq, PartialEq, Hash)]
enum FaultType {
    KafkaConnectionFailure,
    KafkaReadFailure,
    RedisConnectionFailure,
    RedisReadFailure,
    FileOpenFailure,
    FileWriteFailure,
}

#[derive(Parser, Debug)]
#[command(name = "SimulatIOn", version = "1.0", author = "Zaid Humayun")]
struct Args {
    #[arg(short, long)]
    simulate: bool,
}

#[async_trait]
trait IO {
    async fn create_kafka_consumer(
        &mut self,
        group_id: &str,
        broker: &str,
        topic: &str,
        partition: i32,
    ) -> Result<(), Errors>;
    async fn connect_to_redis(&mut self, url: &str) -> Result<(), Errors>;
    async fn open_file(&mut self, path: &Path) -> Result<(), Errors>;
    async fn read_kafka_message(&mut self) -> Result<Option<String>, Errors>;
    async fn get_redis_config(&mut self, key: &str) -> Result<String, Errors>;
    async fn write_to_file(&mut self, data: &str) -> Result<(), Errors>;
}

struct RealIO {
    consumer: Option<StreamConsumer>,
    redis_connection: Option<redis::aio::MultiplexedConnection>,
    file: Option<tokio::fs::File>,
}

impl RealIO {
    fn new() -> Self {
        Self {
            consumer: None,
            redis_connection: None,
            file: None,
        }
    }
}

#[async_trait]
impl IO for RealIO {
    async fn create_kafka_consumer(
        &mut self,
        group_id: &str,
        broker: &str,
        topic: &str,
        partition: i32,
    ) -> Result<(), Errors> {
        let consumer: StreamConsumer = ClientConfig::new()
            .set("group.id", group_id)
            .set("bootstrap.servers", broker)
            .create()
            .map_err(|_| Errors::KafkaConnectionError)?;
        let mut tpl = TopicPartitionList::new();
        tpl.add_partition_offset(topic, partition, rdkafka::Offset::Beginning)
            .map_err(|_| Errors::KafkaConnectionError)?;
        consumer
            .assign(&tpl)
            .map_err(|_| Errors::KafkaConnectionError)?;

        self.consumer = Some(consumer);
        Ok(())
    }

    async fn connect_to_redis(&mut self, url: &str) -> Result<(), Errors> {
        let client = redis::Client::open(url).map_err(|_| Errors::RedisConnectionError)?;
        let connection = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| Errors::RedisConnectionError)?;
        self.redis_connection = Some(connection);
        Ok(())
    }

    async fn open_file(&mut self, path: &Path) -> Result<(), Errors> {
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(path)
            .await
            .map_err(|_| Errors::FileOpenError)?;
        self.file = Some(file);
        Ok(())
    }

    async fn read_kafka_message(&mut self) -> Result<Option<String>, Errors> {
        if let Some(consumer) = &self.consumer {
            let message = consumer.stream().next().await;
            let msg = match message {
                Some(Ok(msg)) => msg
                    .payload()
                    .map(|payload| String::from_utf8_lossy(payload).into_owned()),
                _ => return Err(Errors::NoKafkaMessage),
            };
            return Ok(msg);
        }
        Ok(None)
    }

    async fn get_redis_config(&mut self, key: &str) -> Result<String, Errors> {
        if let Some(redis_conn) = &mut self.redis_connection {
            match redis_conn.get(key).await {
                Ok(value) => Ok(value),
                Err(_) => Err(Errors::RedisKeyRetrievalError),
            }
        } else {
            Err(Errors::RedisConnectionError)
        }
    }

    async fn write_to_file(&mut self, data: &str) -> Result<(), Errors> {
        if let Some(file) = &mut self.file {
            file.write_all(data.as_bytes())
                .await
                .map_err(|_| Errors::FileWriteError)?;
            return Ok(());
        }
        return Err(Errors::FileOpenError);
    }
}

struct SimulatedIO {
    rng: ChaCha8Rng,
    fault_probabilities: HashMap<FaultType, f64>,
    kafka_messages: Vec<String>,
    kafka_attempts: usize,
    kafka_failures: usize,
    redis_data: HashMap<String, String>,
    file_contents: Vec<String>,
}

impl SimulatedIO {
    fn new(seed: u64) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let kafka_messages = vec![
            "simulated_message_1".to_string(),
            "simulated_message_2".to_string(),
            "simulated_message_3".to_string(),
        ];
        let mut redis_data = HashMap::new();
        redis_data.insert(
            "config_key".to_string(),
            "simulated_config_value".to_string(),
        );
        let fault_probabilities = HashMap::from([
            (FaultType::KafkaConnectionFailure, 0.1),
            (FaultType::KafkaReadFailure, 0.1),
            (FaultType::RedisConnectionFailure, 0.1),
            (FaultType::RedisReadFailure, 0.1),
            (FaultType::FileOpenFailure, 0.1),
            (FaultType::FileWriteFailure, 0.1),
        ]);
        let kafka_failures = rng.gen_range(1..5);

        Self {
            rng,
            fault_probabilities,
            kafka_messages,
            redis_data,
            file_contents: Vec::new(),
            kafka_attempts: 0,
            kafka_failures,
        }
    }

    fn should_inject_fault(&mut self, fault_type: &FaultType) -> bool {
        if let Some(&probability) = self.fault_probabilities.get(fault_type) {
            self.rng.gen_bool(probability)
        } else {
            false
        }
    }
}

#[async_trait]
impl IO for SimulatedIO {
    async fn create_kafka_consumer(
        &mut self,
        _group_id: &str,
        _broker: &str,
        _topic: &str,
        _partition: i32,
    ) -> Result<(), Errors> {
        self.kafka_attempts += 1;
        println!("the kafka failures {:?}", self.kafka_failures);
        println!(
            "kafka fault {}",
            self.should_inject_fault(&FaultType::KafkaConnectionFailure)
        );
        if self.should_inject_fault(&FaultType::KafkaConnectionFailure)
            && self.kafka_attempts <= self.kafka_failures
        {
            warn!("Injecting fault for Kafka connection error");
            return Err(Errors::KafkaConnectionError);
        }
        trace!("Not injecting fault for Kafka connection error");
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(())
    }

    async fn connect_to_redis(&mut self, _path: &str) -> Result<(), Errors> {
        if self.should_inject_fault(&FaultType::RedisConnectionFailure) {
            warn!("Injecting fault for Redis connection error");
            return Err(Errors::RedisConnectionError);
        }
        trace!("Not injecting fault for Redis connection error");
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(())
    }

    async fn open_file(&mut self, _path: &Path) -> Result<(), Errors> {
        if self.should_inject_fault(&FaultType::FileOpenFailure) {
            warn!("Injecting fault for file open error");
            return Err(Errors::FileOpenError);
        }
        trace!("Not injecting fault for file open error");
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(())
    }

    async fn read_kafka_message(&mut self) -> Result<Option<String>, Errors> {
        if self.should_inject_fault(&FaultType::KafkaReadFailure) {
            warn!("Injecting fault for Kafka read error");
            return Err(Errors::NoKafkaMessage);
        }
        trace!("Not injecting fault for Kafka read error");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(self.kafka_messages.len() > 0);
        if let Some(message) = self.kafka_messages.choose(&mut self.rng) {
            return Ok(Some(message.clone()));
        }
        return Ok(None);
    }

    async fn get_redis_config(&mut self, key: &str) -> Result<String, Errors> {
        if self.should_inject_fault(&FaultType::RedisReadFailure) {
            warn!("Injecting fault for Redis read error");
            return Err(Errors::RedisKeyRetrievalError);
        }
        trace!("Not injecting fault for Redis read error");
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.redis_data
            .get(key)
            .ok_or(Errors::RedisKeyRetrievalError)
            .cloned()
    }

    async fn write_to_file(&mut self, data: &str) -> Result<(), Errors> {
        if self.should_inject_fault(&FaultType::FileWriteFailure) {
            error!("Injecting fault while writing to file");
            return Err(Errors::FileWriteError);
        }
        trace!("Not injecting fault while writing to a file");
        tokio::time::sleep(Duration::from_millis(50)).await;
        self.file_contents.push(data.to_string());
        Ok(())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    info!("Starting application with args: {:?}", args);

    if args.simulate {
        let seed = match std::env::var("SEED") {
            Ok(seed) => seed.parse::<u64>().unwrap(),
            Err(_) => rand::thread_rng().next_u64(),
        };
        info!("Running simulator with seed {}", seed);
        let mut io = SimulatedIO::new(seed);
        start(&mut io).await;
    } else {
        let mut io = RealIO::new();
        start(&mut io).await;
    }
}

async fn start(io: &mut dyn IO) {
    let io = Arc::new(Mutex::new(io));
    let io_clone = Arc::clone(&io);
    retry_with_backoff(
        || async {
            let mut io = io_clone.lock().unwrap();
            io.create_kafka_consumer("group_id", "localhost:9092", "dummy_topic", 0)
                .await
        },
        5,
        Duration::from_millis(10),
    )
    .await
    .unwrap();
    let mut io = io.lock().unwrap();
    io.connect_to_redis("redis://127.0.0.1").await.unwrap();
    io.open_file(&Path::new("output.txt")).await.unwrap();
    run(*io).await;
}

async fn run(io: &mut dyn IO) {
    let config_key = "config_key";
    loop {
        match io.read_kafka_message().await {
            Ok(Some(message)) => {
                let config_value = match io.get_redis_config(&config_key).await {
                    Ok(msg) => msg,
                    Err(_) => panic!("there was a problem while reading the config from redis"),
                };
                let output = format!("Config: {}, Message: {}\n", config_value, message);
                io.write_to_file(&output)
                    .await
                    .expect("there was a problem while writing to the file");
            }
            Ok(None) => {
                //  need to decide what to do here
            }
            Err(e) => {
                error!("Error reading Kafka message: {:?}", e);
                panic!("there was a problem while reading the Kafka message");
            }
        }
    }
}

async fn retry_with_backoff<F, Fut, T, E>(
    mut operation: F,
    max_retries: usize,
    base_delay: Duration,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let mut retries = 0;
    let mut delay = base_delay;

    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(_) if retries < max_retries => {
                retries += 1;
                let jitter: u64 = rand::thread_rng().gen_range(0..delay.as_millis() as u64);
                let delay_with_jitter = delay + Duration::from_millis(jitter);
                tokio::time::sleep(delay_with_jitter).await;
                delay *= 2;
            }
            Err(err) => return Err(err),
        }
    }
}

// use std::{cell::RefCell, collections::VecDeque, error::Error, future::Future};

// trait Stream {
//     type Item;
// }

// trait AsyncIO {
//     async fn await_future<T>(&self, fut: impl Future<Output = T>) -> T;
//     async fn await_stream<T>(&self, stream: impl Stream<Item = T>) -> Option<T>; //  need to add the futures crate here for the stream trait
// }

// pub trait Kafka {
//     async fn produce(&self, topic: &str, payload: &[u8]);
//     async fn consume(&self, topic: &str) -> Vec<u8>;
// }

// trait Redis {
//     async fn set(&self, key: &str, value: &str) -> Result<(), Box<dyn Error>>;
//     async fn get(&self, key: &str) -> Result<String, Box<dyn Error>>;
//     async fn publish(&self, channel: &str, message: &str) -> Result<(), Box<dyn Error>>;
//     async fn subscribe(&self, channel: &str) -> future::Stream<Item = String>; //   need to import the futures crate here
// }

// struct KafkaClient<'a, IO: AsyncIO> {
//     io: &'a IO,
//     producer: FutureProducer, // some type that implements rdkafka producer
//     consumer: StreamConsumer, // some type that implements rdkafka consumer
// }

// impl<'a, IO: AsyncIO> Kafka for KafkaClient<'a, IO> {
//     async fn produce(&self, topic: &str, payload: &[u8]) {
//         let record = FutureRecord::to(topic).payload(payload);
//         let future = self.producer.send(record);
//         self.io.await_future(future)
//     }
//     async fn consume(&self, topic: &str) -> Vec<u8> {
//         self.consumer.subscribe(topic).unwrap();

//         let stream = self.consumer.stream();
//         let mut messages = Vec::new();
//         while let Some(result) = self.io.await_stream(stream).await {
//             match result {
//                 Ok(msg) => {
//                     if let Some(payload) = msg.payload() {
//                         messages.push(payload);
//                     }
//                 }
//                 Err(e) => eprintln!("Error consuming messages from Kafka {:?}", e),
//             }
//         }

//         messages
//     }
// }

// struct RedisClient<'a, IO: AsyncIO> {
//     io: &'a IO,
//     connection: Connection, //  the connected instance to Redis (probably a TCP socket)
// }

// impl<'a, IO: AsyncIO> Redis for RedisClient<'a, IO> {
//     async fn set(&self, key: &str, value: &str) -> redis::RedisResult<()> {
//         let set_fut = self.connection.set(key, value);
//         self.io.await_future(set_fut).await;
//     }

//     async fn get(&self, key: &str) -> redis::RedisResult<String> {
//         let get_fut = self.connection.get(key);
//         self.io.await_future(get_fut).await;
//     }

//     async fn publish(&self, channel: &str, message: &str) -> redis::RedisResult<()> {
//         let pub_fut = self.connection.publish(channel, message);
//         self.io.await_future(pub_fut).await.unwrap();
//     }

//     async fn subscribe(&self, channel: &str) -> impl futures::Stream<Item = String> {
//         let mut pubsub = self.connection.as_pubsub();
//         pubsub.subscribe(channel).await.unwrap();

//         self.io
//             .await_stream(pubsub.on_message().map(|msg| msg.get_payload().unwrap()))
//     }
// }

// pub struct IO {
//     //  what fields do I need here?
// }

// impl IO {
//     pub fn new() -> Self {
//         Self {}
//     }
// }

// impl AsyncIO for IO {
//     async fn await_future<T>(&self, fut: impl Future<Output = T>) -> T {
//         todo!()
//         //  the actual implementation goes here
//     }
//     async fn await_stream<T>(&self, stream: impl Stream<Item = T>) -> Option<T> {
//         todo!()
//         //  the actual implementation goes here
//     }
// }

// pub struct SimulatedIO {
//     //  what do I need here?
//     inner: Box<dyn AsyncIO>,
//     rng: RefCell<ChaCha8Rng>,
// }

// impl SimulatedIO {
//     fn new(seed: u64) -> Self {
//         let inner = IO::new();
//         let rng = ChaCha8Rng::seed_from_u64(seed);
//         Self { inner, rng }
//     }
// }

// impl AsyncIO for SimulatedIO {
//     async fn await_future<T>(&self, fut: impl Future<Output = T>) -> T {
//         todo!()
//         //  return simulated responses here
//     }
//     async fn await_stream<T>(&self, stream: impl Stream<Item = T>) -> Option<T> {
//         todo!()
//         //  return simulated responses here
//     }
// }

// struct FaultInjector {
//     rng: ChaCha8Rng,
//     connection_failure_prob: f64,
//     packet_loss_prob: f64,
//     out_of_order_prob: f64,
//     intermittent_disconnect_prob: f64,
// }

// impl FaultInjector {
//     pub fn new(
//         rng: ChaCha8Rng,
//         connection_failure_prob: f64,
//         packet_loss_prob: f64,
//         out_of_order_prob: f64,
//         intermittent_disconnect_prob: f64,
//     ) -> Self {
//         Self {
//             rng,
//             connection_failure_prob,
//             packet_loss_prob,
//             out_of_order_prob,
//             intermittent_disconnect_prob,
//         }
//     }

//     pub fn should_inject_connection_failure(&mut self) -> bool {
//         self.rng.gen_bool(self.connection_failure_prob)
//     }

//     pub fn should_inject_packet_loss(&mut self) -> bool {
//         self.rng_gen_bool(self.packet_loss_prob)
//     }

//     pub fn should_inject_out_of_order(&mut self) -> bool {
//         self.rng_gen_bool(self.out_of_order_prob)
//     }

//     pub fn should_inject_intermittent_disconnect(&mut self) -> bool {
//         self.rng_gen_bool(self.intermittent_disconnect_prob)
//     }
// }

// struct SimulatedKafka<'a, IO: AsyncIO> {
//     inner: KafkaClient<'a, IO>,
//     fault_injector: FaultInjector,
//     incoming_buffer: VecDeque<Vec<u8>>,
// }

// impl Kafka for SimulatedKafka<'_, IO> {
//     async fn produce(&self, topic: &str, payload: &[u8]) {
//         if self.fault_injector.should_inject_packet_loss() {
//             println!("Kafka fault injected: packet loss on message to topic: {topic}");
//             return ();
//         }

//         if self.fault_injector.should_inject_intermittent_disconnect() {
//             println!("Kafka fault injected: disconnected from downstream Kafka");
//             //  should return an error here indicating that downstream kafka was disconnected
//             return ();
//         }

//         self.inner.produce(topic, payload);
//         todo!()
//     }

//     async fn consume(&self, topic: &str) -> Vec<u8> {
//         let message = self.inner.consume(topic).await;
//         if self.fault_injector.should_inject_out_of_order() {
//             self.incoming_buffer.push_back(message);
//             if !self.incoming_buffer.is_empty() {
//                 let delayed_message = self.incoming_buffer.pop_front().unwrap();
//                 println!(
//                     "Delivering delayed (out-of-order) message: {}",
//                     delayed_message
//                 );
//                 return Some(delayed_message);
//             }
//             None
//         } else {
//             if !self.incoming_buffer.is_empty() {
//                 Some(self.incoming_buffer.pop_front().unwrap())
//             } else {
//                 Some(message)
//             }
//         }
//         todo!()
//     }
// }

// struct SimulatedRedis<'a, IO: AsyncIO> {
//     inner: RedisClient<'a, IO>,
//     fault_injector: FaultInjector,
// }

// impl Redis for SimulatedRedis<'_, IO> {
//     async fn set(&self, key: &str, value: &str) -> Result<(), Box<dyn Error>> {
//         if self.fault_injector.should_inject_connection_failure() {
//             println!("Redis fault injected: connection failure");
//             return Err(Box::new(std::io::Error::new(
//                 std::io::ErrorKind::ConnectionRefused,
//                 "Connection refused",
//             )));
//         }

//         self.inner.set(key, value).await;
//         Ok(())
//     }

//     async fn get(&self, key: &str) -> Result<String, Box<dyn Error>> {
//         if self.fault_injector.should_inject_connection_failure() {
//             println!("Redis fault injected: connection failure");
//             return Err(Box::new(std::io::Error::new(
//                 std::io::ErrorKind::ConnectionRefused,
//                 "Connection refused",
//             )));
//         }

//         self.inner.get(key).await;
//         Ok(())
//     }

//     async fn publish(&self, channel: &str, message: &str) -> Result<(), Box<dyn Error>> {
//         if self.fault_injector.should_inject_connection_failure() {
//             println!("Redis fault injected: connection failure");
//             return Err(Box::new(std::io::Error::new(
//                 std::io::ErrorKind::ConnectionRefused,
//                 "Connection refused",
//             )));
//         }

//         self.inner.publish(channel, message).await;
//         Ok(())
//     }

//     async fn subscribe(&self, channel: &str) -> future::Stream<Item = String> {
//         if self.fault_injector.should_inject_connection_failure() {
//             println!("Redis fault injected: connection failure");
//             return Err(Box::new(std::io::Error::new(
//                 std::io::ErrorKind::ConnectionRefused,
//                 "Connection refused",
//             )));
//         }

//         self.inner.subscribe(channel).await;
//         Ok(())
//     }
// }

// /// How do I partition the machine's cores such that I can run a separate event loop per core(separate Tokio or Glommio runtime) and
// /// run each loop deterministically
// /// The simplest way would be for me to hash the customer id % by the number of cores so each customer id is assigned to a specific core
// /// but this doesn't account for the fact that incoming traffic is very skewed. Need to think of a better partitioning strategy.
// fn main() {
//     println!("Hello, world!");

//     //  When running the application, the entire thing needs to run in a loop, consuming messages from Kakfa, reading config from Redis
//     //  processing the required data and publishing to a downstream Kafka
//     //  When running the simulation, the entire loop will run with the simulated mocks, and the entire thing should be reproducible from the seed

//     let seed = 1234; //  generate a random seed here
//     let connection_failure_prob = 0.1; //  10% chance of connection failure
//     let packet_loss_prob = 0.15; //  15% chance for packet loss
//     let out_of_order_prob = 0.1; // 10% chance for out-of-order delivery
//     let intermittent_disconnect_prob = 0.2; // 20% chance for intermittent disconnect
//     let rng = ChaCha8Rng::seed_from_u64(seed);

//     let fault_injector = FaultInjector::new(
//         rng,
//         connection_failure_prob,
//         packet_loss_prob,
//         out_of_order_prob,
//         intermittent_disconnect_prob,
//     );
//     let simulated_io = SimulatedIO::new(seed);
//     //  create a kafka & redis client
//     //  create a simulated kafka and redis client here and feed in the actual clients as dependencies
//     //  run the application logic in a loop
// }

// // My question around why the rng works even in probabilistic code like should_inject_connection_failure was answered quite well in this link [here](https://chatgpt.com/share/67290aae-5368-800e-8399-c2a6b6b68010)
