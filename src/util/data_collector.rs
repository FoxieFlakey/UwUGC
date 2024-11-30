// Its job is to receive record containg N numberic fields,
// and broadcast to each consumers

use std::{sync::{mpsc, Arc, Mutex}, thread::{self, JoinHandle}};

pub trait Consumer<T: Send + 'static>: Send {
  fn consume(&mut self, data: &T);
}

pub struct DataCollector<T: Send + 'static> {
  consumers: Arc<Mutex<Vec<Box<dyn Consumer<T>>>>>,
  thread: Option<JoinHandle<()>>,
  sender: mpsc::SyncSender<Message<T>>
}

enum Message<T: Send> {
  Shutdown,
  ProcessData(T)
}

impl<T: Send + 'static> DataCollector<T> {
  pub fn new(queue_size: usize) -> Self {
    let (sender, receiver) = mpsc::sync_channel(queue_size);
    let consumers = Arc::new(Mutex::new(Vec::new()));
    let mut new_self = Self {
      consumers,
      sender,
      thread: None
    };
    
    let consumers2 = new_self.consumers.clone();
    new_self.thread = Some(thread::spawn(move || {
      while let Message::ProcessData(data) = receiver.recv().unwrap() {
        let mut list_of_consumers = consumers2.lock().unwrap();
        for consumer in list_of_consumers.iter_mut() {
          consumer.consume(&data);
        }
      }
    }));
    return new_self;
  }
  
  pub fn put_data(&self, data: T) {
    self.sender.send(Message::ProcessData(data)).unwrap();
  }
  
  pub fn add_consumer(&self, consumer: Box<dyn Consumer<T>>) {
    self.consumers.lock().unwrap().push(consumer);
  }
  
  pub fn add_consumer_fn(&self, consumer: impl FnMut(&T) + Send + 'static) {
    struct ConsumerFromFn<T: Send + 'static> {
      func: Box<dyn FnMut(&T) + Send>
    }
    
    impl<T: Send + 'static > Consumer<T> for ConsumerFromFn<T> {
      fn consume(&mut self, data: &T) {
        return (self.func)(data);
      }
    }
    
    self.add_consumer(Box::new(ConsumerFromFn {
      func: Box::new(consumer)
    }));
  }
}

impl<T: Send> Drop for DataCollector<T> {
  fn drop(&mut self) {
    self.sender.send(Message::Shutdown).unwrap();
    self.thread.take().unwrap().join().unwrap();
  }
}

