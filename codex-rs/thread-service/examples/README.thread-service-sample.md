# thread-service-sample

这个示例原先是独立 crate `thread-service-sample`，现在收缩为
`thread-service` 的 example。

运行方式：

```sh
cargo run -p thread-service --features test-support --example thread-service-sample -- "Say hello"
```

指定模型：

```sh
cargo run -p thread-service --features test-support --example thread-service-sample -- --model gpt-5.2 "Say hello"
```

从 stdin 读入 prompt：

```sh
printf 'Say hello\n' | cargo run -p thread-service --features test-support --example thread-service-sample
```
