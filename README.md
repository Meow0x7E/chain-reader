# chain-reader

A Rust library for sequentially chaining multiple [`Read`] instances with configurable error handling.

## Features

- **Dynamic Reader Chaining**: Combine multiple readers (including from iterators) into a single sequential read pipeline
- **Configurable Error Handling**: Define custom strategies for handling I/O errors using the [`ErrorAction`] enum
- **Automatic Advancement**: Automatically progresses to the next reader on EOF
- **FIFO Processing**: Processes readers in first-in-first-out order

## Comparison with `std::io::Chain`

- Supports dynamic addition of readers (not limited to two fixed readers)
- Provides flexible error handling strategies
- Handles both single readers and iterators of readers
- Automatically advances on EOF (no need for manual checking)

## Usage

```rust
use chain_reader::{ChainReader, ErrorAction};
use std::io::{self, Read};

let mut chain = ChainReader::new(|e| match e.kind() {
    io::ErrorKind::Interrupted => ErrorAction::Retry,
    _ => ErrorAction::RetryAndSkip,
});

chain.push(io::stdin());
chain.push_iter(vec![
    io::Cursor::new("hello "),
    io::Cursor::new("world!"),
]);

let mut content = String::new();
chain.read_to_string(&mut content)?;
```
