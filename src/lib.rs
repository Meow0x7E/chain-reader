use std::{
    collections::VecDeque,
    io::{self, Read},
};

/// A sequential chaining reader that combines multiple [`Read`] instances with configurable error handling.
///
/// # Behavior
///
/// - Readers are consumed in FIFO (first-in, first-out) order.
/// - Automatically switches to the next reader when the current one reaches EOF (returns `Ok(0)`).
/// - Allows custom error handling to decide whether to retry or skip on I/O errors.
///
/// # Comparison with [`io::Chain`]
///
/// - Supports a dynamic queue of readers instead of a fixed pair.
/// - Configurable error handling strategies: Retry or Skip
/// - Automatically advances to the next reader on EOF.
///
/// ---
///
/// 顺序管道读取器，将多个 [`Read`] 实例组合为具有可配置错误处理的顺序读取管道。
///
/// # 行为特性：
///
/// - 按先进先出顺序消费读取器。
/// - 当前读取器到达 EOF 时（返回 `Ok(0)`）自动切换至下一个。
/// - 允许自定义错误处理策略，决定在 I/O 错误时重试或跳过。
///
/// # 与 [`io::Chain`] 的区别：
///
/// - 支持动态读取器队列而非固定两个。
/// - 可配置错误处理策略：重试或跳过。
/// - EOF 时自动管理读取器切换。
#[derive(Debug, Clone)]
pub struct ChainReader<R, F>
where
    R: Read,
    F: FnMut(io::Error) -> ErrorAction,
{
    /// Reader queue managed as FIFO buffer (first-in, first-out).
    ///
    /// Implemented with [`VecDeque`] for O(1) pop-front operations. The front reader
    /// remains active until it returns EOF (`Ok(0)`), encounters an error handled by [`ErrorAction::Skip`],
    /// or is explicitly removed.
    ///
    /// ---
    ///
    /// 先进先出读取器队列。
    ///
    /// 使用 [`VecDeque`] 实现 O(1) 复杂度前端弹出操作。队列首部的读取器保持激活状态，
    /// 直到返回 EOF（`Ok(0)`）、遇到由 [`ErrorAction::Skip`] 处理的错误，或被显式移除。
    fifo: VecDeque<R>,
    /// Error handling callback for read operations.
    ///
    /// When a read error occurs, this callback is invoked with the encountered [`io::Error`].
    /// The chaining behavior is determined by the returned [`ErrorAction`]:
    /// - [`ErrorAction::Retry`]: Propagate the error but retain the current reader for retries.
    /// - [`ErrorAction::Skip`]: Discard the current reader and proceed to the next.
    ///
    /// ---
    ///
    /// 读取操作错误处理回调。
    ///
    /// 当读取错误发生时，此回调函数接收遇到的 [`io::Error`]，
    /// 并通过返回的 [`ErrorAction`] 决定链式读取行为：
    /// - [`ErrorAction::Retry`]：传播错误但保留当前读取器以供重试。
    /// - [`ErrorAction::Skip`]：跳过当前读取器并继续下一个。
    handle: F,
}

impl<R, F> ChainReader<R, F>
where
    R: Read,
    F: FnMut(io::Error) -> ErrorAction,
{
    /// Creates a new `ChainReader` with the given reader queue and error handler.
    ///
    /// The readers will be consumed in the order they appear in the `readers` queue.
    /// The `handle` callback determines behavior on I/O errors during reading.
    ///
    /// # Parameters
    /// - `readers`: Queue of readers to process sequentially
    /// - `handle`: Error handling strategy callback
    ///
    /// # Examples
    /// ```
    /// use std::collections::VecDeque;
    /// use std::io;
    /// use chain_reader::{ChainReader, ErrorAction};
    ///
    /// let mut readers = VecDeque::new();
    /// readers.push_back("hello".as_bytes());
    /// let error_handler = |e: io::Error| ErrorAction::Skip;
    /// let chain = ChainReader::new(readers, error_handler);
    /// ```
    ///
    /// ---
    ///
    /// 使用指定的读取器队列和错误处理回调创建新的 `ChainReader`
    ///
    /// 读取器将按照队列中的顺序被消费。`handle` 回调用于决定读取时遇到 I/O 错误的处理策略
    ///
    /// # 参数
    /// - `readers`: 要顺序处理的读取器队列
    /// - `handle`: 错误处理策略回调
    ///
    /// # 示例
    /// ```
    /// use std::collections::VecDeque;
    /// use std::io;
    /// use chain_reader::{ChainReader, ErrorAction};
    ///
    /// let mut readers = VecDeque::new();
    /// readers.push_back("hello".as_bytes());
    /// let error_handler = |e: io::Error| ErrorAction::Skip;
    /// let chain = ChainReader::new(readers, error_handler);
    /// ```
    pub fn new(readers: VecDeque<R>, handle: F) -> Self {
        ChainReader {
            fifo: readers,
            handle,
        }
    }

    /// Appends a reader to the end of the processing queue
    ///
    /// The reader will be processed after all existing readers in the queue
    /// have either reached EOF or been skipped due to errors.
    ///
    /// # Parameters
    /// - `reader`: Reader to add to the end of the queue
    ///
    /// # Examples
    /// ```
    /// use std::collections::VecDeque;
    /// use std::io;
    /// use chain_reader::{ChainReader, ErrorAction};
    ///
    /// let mut chain = ChainReader::new(VecDeque::new(), |e| ErrorAction::Skip);
    /// chain.push_back(std::io::empty());
    /// ```
    ///
    /// ---
    ///
    /// 将读取器追加到处理队列末尾
    ///
    /// 该读取器将在队列中所有现有读取器处理完毕（到达 EOF 或因为错误被跳过）后进行处理
    ///
    /// # 参数
    /// - `reader`: 要添加到队列末尾的读取器
    ///
    /// # 示例
    /// ```
    /// use std::collections::VecDeque;
    /// use std::io;
    /// use chain_reader::{ChainReader, ErrorAction};
    /// let mut chain = ChainReader::new(VecDeque::new(), |e| ErrorAction::Skip);
    /// chain.push_back(std::io::empty());
    /// ```
    pub fn push_back(&mut self, reader: R) {
        self.fifo.push_back(reader);
    }

    /// Removes and returns the front reader from the queue
    ///
    /// Used internally when advancing to the next reader. Maintains
    /// O(1) time complexity for queue operations.
    ///
    /// ---
    ///
    /// 移除并返回队列前端的读取器
    ///
    /// 内部用于切换到下一个读取器，保持队列操作的 O(1) 时间复杂度
    fn pop_front(&mut self) -> Option<R> {
        self.fifo.pop_front()
    }
}

impl<R, F> Read for ChainReader<R, F>
where
    R: Read,
    F: FnMut(io::Error) -> ErrorAction,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let reader = match self.fifo.front_mut() {
                Some(it) => it,
                None => return Ok(0),
            };

            match reader.read(buf) {
                Ok(0) => {
                    let _ = self.pop_front();
                }
                Ok(it) => {
                    return Ok(it);
                }
                Err(it) => {
                    let r: ErrorAction = (self.handle)(it);
                    match r {
                        ErrorAction::Retry(it) => return Err(it),
                        ErrorAction::Skip => {
                            let _ = self.pop_front();
                        }
                    }
                }
            }
        }
    }
}

/// Defines the action to take when a read error occurs.
///
/// ---
///
/// 定义读取错误发生时的处理动作。
pub enum ErrorAction {
    /// Propagate the error but retain the current reader.
    ///
    /// The next read operation will retry the same reader. This is useful for
    /// transient errors where retrying might succeed.
    ///
    /// ---
    ///
    /// 传播错误但保留当前读取器。
    ///
    /// 下次读取操作将继续尝试同一读取器。适用于可能通过重试解决的临时错误。
    Retry(io::Error),
    /// Discard the current reader and proceed to the next one.
    ///
    /// The problematic reader is removed from the queue immediately.
    ///
    /// ---
    ///
    /// 丢弃当前读取器并继续下一个。
    ///
    /// 有问题的读取器将立即从队列中移除。
    Skip,
}
