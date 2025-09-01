use std::{
    collections::VecDeque,
    io::{self, Read},
    marker,
};

/// Defines the action to take when a read error occurs.
///
/// This enum allows fine-grained control over how `ChainReader` handles I/O errors
/// during read operations. Each variant represents a different strategy for
/// error recovery or propagation.
#[derive(Debug)]
pub enum ErrorAction {
    /// Propagate the error but retain the current reader.
    ///
    /// The next read operation will retry the same reader. This is useful for
    /// transient errors where retrying might succeed (e.g., `io::ErrorKind::Interrupted`).
    Raise(io::Error),

    /// Propagate the error and skip to the next reader.
    ///
    /// The problematic reader is removed from the queue and the error is returned.
    /// This is useful for fatal errors that cannot be recovered from.
    RaiseAndSkip(io::Error),

    /// Silently retry the current reader without propagating the error.
    ///
    /// The error is swallowed and the same reader will be retried on the next read.
    /// This is useful for errors that should be handled transparently.
    Retry,

    /// Silently skip to the next reader without propagating the error.
    ///
    /// The problematic reader is removed from the queue without returning an error.
    /// This is useful for non-critical errors where skipping is acceptable.
    RetryAndSkip,
}

/// Internal enum representing different sources of readers in the chain.
///
/// This enum is used internally by `ChainReader` to manage both single readers
/// and iterators that produce multiple readers. It allows the chain to handle
/// both types of reader sources uniformly while maintaining proper FIFO ordering.
///
/// # Variants
///
/// - `Single(R)`: A single reader instance
/// - `Multi(I)`: An iterator that produces multiple reader instances
///
/// This type is not part of the public API and is for internal use only.
#[derive(Debug)]
enum ReaderSource<R, I>
where
    R: Read,
    I: Iterator<Item = R>,
{
    /// A single reader instance
    Single(R),
    /// An iterator that produces multiple reader instances
    Multi(I),
}

/// A sequential chaining reader that combines multiple [`Read`] instances with configurable error handling.
///
/// # Behavior
///
/// - Readers are consumed in FIFO (first-in, first-out) order.
/// - Automatically switches to the next reader when the current one reaches EOF (returns `Ok(0)`).
/// - Allows custom error handling to decide whether to retry or skip on I/O errors.
/// - Supports both single readers and iterators that produce readers.
///
/// # Comparison with [`io::Chain`]
///
/// - Supports a dynamic queue of readers instead of a fixed pair.
/// - Configurable error handling strategies (see [`ErrorAction`] for details)
/// - Automatically advances to the next reader on EOF.
/// - Handles both single readers and iterators of readers.
///
/// # Examples
///
/// ```
/// use chain_reader::{ChainReader, ErrorAction};
/// use std::io::{self, Read};
///
/// // Define a custom reader type that can handle both stdin and files
/// enum Readers {
///     Stdin,
///     File(std::fs::File),
///     Cursor(io::Cursor<String>),
/// }
///
/// impl Read for Readers {
///     fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
///         match self {
///             Readers::Stdin => io::stdin().read(buf),
///             Readers::File(it) => it.read(buf),
///             Readers::Cursor(it) => it.read(buf),
///         }
///     }
/// }
///
/// fn main() -> io::Result<()> {
///     // Create a ChainReader that starts with stdin
///     let mut chain = ChainReader::new(
///         |e| match e.kind() {
///             io::ErrorKind::Interrupted => ErrorAction::Retry,
///             _ => ErrorAction::RetryAndSkip,
///         },
///     );
///
///
///     chain.push(Readers::Stdin);
///     // Add a file to the chain
///     chain.push(Readers::File(std::fs::File::open("Cargo.toml")?));
///     chain.push_iter(vec![
///         Readers::Cursor(io::Cursor::new("hello ".to_string())),
///         Readers::Cursor(io::Cursor::new("world!".to_string())),
///     ]);
///
///     // Read from the chain - first from stdin, then from the file
///     let mut content = Vec::new();
///     chain.read_to_end(&mut content)?;
///
///     Ok(())
/// }
/// ```
pub struct ChainReader<'a, R, F, I = std::iter::Empty<R>>
where
    R: Read,
    I: Iterator<Item = R>,
    F: FnMut(io::Error) -> ErrorAction,
{
    /// Current active reader
    current: Option<R>,
    /// Queue of reader sources waiting to be processed
    sources: VecDeque<ReaderSource<R, I>>,
    /// Error handling callback for read operations.
    ///
    /// When a read error occurs, this callback is invoked with the encountered [`io::Error`].
    /// The chaining behavior is determined by the returned [`ErrorAction`] value.
    /// See the [`ErrorAction`] documentation for available error handling strategies.
    error_handle: F,
    _marker: marker::PhantomData<&'a R>,
}

impl<R, F, I> ChainReader<'_, R, F, I>
where
    R: Read,
    I: Iterator<Item = R>,
    F: FnMut(io::Error) -> ErrorAction,
{
    /// Creates a new `ChainReader` with the given error handler and an empty reader queue.
    ///
    /// Readers can be added to the queue using the [`Self::push`] and [`Self::push_iter`] methods.
    /// The `error_handle` callback determines behavior on I/O errors during reading.
    ///
    /// # Parameters
    /// - `error_handle`: Error handling strategy callback
    pub fn new(error_handle: F) -> Self {
        Self {
            current: None,
            sources: VecDeque::new(),
            error_handle,
            _marker: marker::PhantomData,
        }
    }

    /// Replaces the current error handler with a new one.
    ///
    /// This method allows changing the error handling strategy at runtime.
    ///
    /// # Parameters
    /// - `error_handle`: New error handling strategy callback
    pub fn replace_handle(&mut self, error_handle: F) {
        self.error_handle = error_handle;
    }

    /// Appends a single reader to the end of the processing queue.
    ///
    /// The reader will be processed after all existing readers in the queue
    /// have either reached EOF or been skipped due to errors.
    ///
    /// # Parameters
    /// - `reader`: Reader to add to the end of the queue
    pub fn push(&mut self, reader: R) {
        self.sources.push_back(ReaderSource::Single(reader));
    }

    /// Appends an iterator of readers to the end of the processing queue.
    ///
    /// The readers produced by the iterator will be processed in order after
    /// all existing readers in the queue have been processed.
    ///
    /// # Parameters
    /// - `iter`: Iterator that produces readers to add to the end of the queue
    pub fn push_iter<II: IntoIterator<IntoIter = I>>(&mut self, iter: II) {
        self.sources
            .push_back(ReaderSource::Multi(iter.into_iter()));
    }

    /// Advances to the next reader in the queue.
    ///
    /// Returns `true` if a new reader was successfully advanced to,
    /// or `false` if there are no more readers in the queue.
    ///
    /// # Internal Implementation
    ///
    /// This method handles both single readers and iterator-based readers,
    /// ensuring proper FIFO processing order.
    fn next(&mut self) -> bool {
        while let Some(source) = self.sources.pop_front() {
            match source {
                ReaderSource::Single(reader) => {
                    self.current = Some(reader);
                    return true;
                }
                ReaderSource::Multi(mut iter) => {
                    if let Some(reader) = iter.next() {
                        self.current = Some(reader);
                        self.sources.push_front(ReaderSource::Multi(iter));
                        return true;
                    }
                }
            }
        }
        self.current = None;
        false
    }
}

impl<R, F, I> Read for ChainReader<'_, R, F, I>
where
    R: Read,
    I: Iterator<Item = R>,
    F: FnMut(io::Error) -> ErrorAction,
{
    /// Reads data from the current reader, handling errors and EOF according to the configured strategy.
    ///
    /// This implementation follows the chain reading pattern:
    /// 1. Attempt to read from the current reader
    /// 2. On success, return the read data
    /// 3. On EOF, automatically advance to the next reader
    /// 4. On error, invoke the error handler to determine the appropriate action
    ///
    /// The method will continue this process until either:
    /// - Data is successfully read
    /// - All readers are exhausted (returning Ok(0))
    /// - An unhandled error occurs (returning Err)
    ///
    /// # Parameters
    /// - `buf`: Buffer to read data into
    ///
    /// # Returns
    /// - `Ok(n)`: Successfully read `n` bytes
    /// - `Err(e)`: IO error that couldn't be handled by the error strategy
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.current.is_none() && !self.next() {
                return Ok(0);
            }

            match self.current.as_mut().unwrap().read(buf) {
                Ok(0) => {
                    if !self.next() {
                        return Ok(0);
                    }
                }
                Ok(n) => return Ok(n),
                Err(e) => match (self.error_handle)(e) {
                    ErrorAction::Raise(e) => return Err(e),
                    ErrorAction::RaiseAndSkip(e) => {
                        self.next();
                        return Err(e);
                    }
                    ErrorAction::Retry => {}
                    ErrorAction::RetryAndSkip => {
                        self.next();
                    }
                },
            }
        }
    }
}
