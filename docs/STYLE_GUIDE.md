# Rust Code Style Guide

These conventions favor clarity, explicit invariants, and maintainable
boundaries. They are defaults for code being touched, not a reason to mix
unrelated refactors into a behavioral change.

## Organize code around responsibilities

Give each module one coherent responsibility. Keep dependencies pointing
toward stable domain concepts: boundary adapters may depend on domain types,
but domain modules should not depend on transports, runtimes, command-line
parsers, or presentation frameworks.

Within a substantial `impl`, group methods by responsibility or call flow.
Keep a helper close to the operations it supports unless it is shared broadly
enough to deserve its own module.

Avoid extracting an abstraction merely to reduce line count. Extract it when
it gives a concept a useful name, protects an invariant, or removes policy
duplication that could otherwise drift.

## Keep the public surface narrow

Expose only the types and operations callers need. Prefer private modules with
deliberate re-exports over making an implementation hierarchy public. Use the
narrowest visibility that permits the intended collaboration.

```rust
mod implementation;

pub use implementation::Service;
```

Public APIs should communicate supported behavior rather than accidental
representation details. Treat widening visibility as an API decision, not as
the easiest way around an ownership or module-boundary problem.

## Use types that express meaning

Prefer a meaningful type over a primitive when a value has identity,
validation rules, units, or a restricted set of operations.

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct RecordId(u64);
```

Use distinct types for concepts that should not be interchangeable, even when
their storage representation is identical. Keep raw integers and strings at
representation boundaries rather than spreading them through domain code.

Small fieldless enums may implement `Copy` and be passed by value. Larger
values should normally be borrowed unless the callee may retain them.

## Convert representations at boundaries

Store data in the representation native to its domain and convert when
crossing into another system. Centralizing conversions avoids repeated work
and prevents boundary-specific failure states from leaking inward.

Validate incoming representations once. After conversion, internal code
should be able to rely on the resulting type's invariants rather than repeat
the same checks.

## Protect invariants with narrow construction

Keep fields private when construction or mutation must preserve an invariant.
Expose constructors and the smallest useful set of accessors instead of making
representation details public for convenience.

```rust
struct NonEmptyName(String);

impl NonEmptyName {
    fn new(value: String) -> Result<Self, NameError> {
        if value.is_empty() {
            return Err(NameError::Empty);
        }

        Ok(Self(value))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}
```

If several fields must change together, provide one operation that performs
the transition atomically instead of exposing independent setters.

## Choose names that communicate outcomes

Name an operation after what it returns or establishes. Avoid vague verbs such
as `process`, `handle`, `populate`, or `generate` when a more precise name is
available. Avoid abbreviations when the full word materially improves
understanding; retain abbreviations fixed by an external API.

Use `err` for a short-lived local error binding and `error` for a field or a
longer-lived semantic value.

For an intentionally unused trait parameter, use `_` when its name adds no
local information. Use an underscore-prefixed name when retaining its meaning
helps explain the implementation or anticipates imminent use.

```rust
fn on_event(&self, _: &Context, event: Event, _: EventFlags) {
    self.record(event);
}
```

## Treat rustfmt as the layout authority

Run rustfmt and accept its stable output. Do not maintain hand formatting that
rustfmt immediately rewrites. Improve difficult layout through better names,
types, and smaller expressions rather than manual alignment.

Group imports into standard-library, external-crate, and current-crate blocks.
Within those groups, let rustfmt determine layout.

## Keep the successful path flat

Use early returns, `?`, and `let ... else` when they make the main operation
read from top to bottom.

```rust
let Some(record) = records.get(&id) else {
    return Err(LookupError::NotFound(id));
};

let value = decode(record)?;
store(value)?;
```

Keep a `match` when its alternatives represent meaningfully different domain
outcomes. Do not replace a clear exhaustive match with indirect machinery
merely to reduce its size.

## Separate fallible work from one-shot effects

Boundary methods should translate inputs, delegate fallible work to ordinary
operations, and perform one final externally visible effect. Helpers should
return values and errors rather than own response objects, transactions, or
other one-shot capabilities.

```rust
match self.load_record(id) {
    Ok(record) => response.send(record),
    Err(err) => response.fail(err.code()),
}
```

This structure makes success and failure paths auditable and prevents helpers
from accidentally performing an effect twice.

## Use ownership to communicate lifetime

Borrow when an operation only observes a value. Consume a value when the
callee may retain it or when ownership transfer is the point of the operation.
Use `Arc` when data must remain valid independently across concurrent
operations.

Do not clone solely to satisfy an unnecessarily broad interface. Conversely,
do not return a reference tied to a lock guard when a small owned value or an
`Arc` would make the lifetime and concurrency behavior clearer.

Types that acquire resources should define who releases them and what happens
on partial failure. Prefer RAII when cleanup can be expressed by ownership;
use explicit lifecycle operations when an external protocol controls release.

## Make errors semantic and standard

Error variants should describe meaningful failure categories rather than use
generic string buckets. Implement `std::error::Error`, retain useful sources,
and translate errors at the boundary where the required external vocabulary is
known.

```rust
#[derive(Debug, thiserror::Error)]
enum LoadError {
    #[error("record {0:?} was not found")]
    NotFound(RecordId),

    #[error("storage request failed")]
    Storage(#[source] StorageError),
}
```

Avoid unchecked `unwrap()` and `expect()` in production paths. Propagate an
ordinary failure, map it into a semantic error, or document why the state is a
proven invariant. Convenience combinators such as `unwrap_or` are acceptable
when they do not panic and their fallback expresses the intended policy.

## Check numeric boundaries

Use checked arithmetic and fallible conversions when values cross integer
representations or participate in externally visible offsets, identifiers,
sizes, or counters.

```rust
let size = u64::try_from(bytes.len()).map_err(|_| EncodeError::SizeOverflow)?;
let next = current.checked_add(1).ok_or(IdError::Exhausted)?;
```

Give conversion failures a meaning appropriate to the operation. Do not add a
broad conversion from every integer error when different boundaries require
different handling.

## Make concurrency policy explicit

Keep critical sections small and avoid holding synchronous locks across slow,
fallible, or asynchronous work. Clone an owned handle while holding the lock,
then release the guard before using it.

```rust
let value = {
    let state = self.state.lock()?;
    state.entries.get(&key).cloned()
};
```

Decide deliberately whether poisoned state is recoverable. Represent lock
failure rather than silently assuming it is impossible.

For reference-counted lifecycle state, document which events increment and
decrement each counter, which combinations permit eviction, and whether
cleanup must occur even when the surrounding operation reports an error.

## Document every unsafe proof

Keep unsafe blocks as small as practical and precede them with a `// SAFETY:`
comment explaining why every relevant precondition holds. State the proof; do
not merely restate the operation.

```rust
// SAFETY: `pointer` is non-null, aligned, and valid for reads of `length`
// initialized bytes for the duration of the returned slice.
let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
```

For an `unsafe fn`, include a `# Safety` documentation section describing the
obligations imposed on callers.

## Use structured, contextual diagnostics

Use structured fields with stable names and representations. Keep the message
short and constant; record values such as the operation, identity, source
error, and outcome as fields rather than interpolating them into prose.

Choose levels according to meaning and urgency:

| Level | General meaning |
| --- | --- |
| `error` | An internal invariant was violated or state may be corrupted and requires investigation. |
| `warn` | An operation failed because of a recoverable external or operational problem. |
| `info` | A low-volume lifecycle event useful during normal operation. |
| `debug` | An expected negative outcome or state transition useful during diagnosis. |
| `trace` | High-volume request-by-request detail that is normally disabled. |

Log a failure once, at the layer that handles its outcome and has enough
context to describe it. Lower layers should normally return typed errors
instead of logging and returning the same failure. Logging must not alter
control flow, error mapping, or cleanup.

Use `%value` for `Display` and `?value` for `Debug`. Prefer native recording
for numbers and booleans. Keep a field's representation consistent across
events so structured consumers do not see the same field alternate between a
number and a formatted string.

```rust
tracing::warn!(
    operation = "load_record",
    record_id = id.0,
    error = %err,
    "operation failed",
);
```

Do not record credentials, authorization material, configuration secrets, or
sensitive payloads. Do not automatically derive `Debug` for a type that owns
sensitive or potentially large data. If diagnostics require it, implement a
deliberate representation that reports safe metadata such as identity and
length without including the payload.

Libraries may emit diagnostics but should not install a global subscriber.
The executable owns subscriber selection, filtering, formatting, and output,
and should prefer fallible initialization.

## Keep application composition explicit and fallible

Construct long-lived dependencies in one visible place. Make configuration an
input, keep framework-specific parsing at the executable boundary, and convert
it into internal configuration types.

```rust
fn run(config: Config) -> Result<(), ApplicationError> {
    let runtime = Runtime::new()?;
    let service = Service::new(config, runtime.handle().clone())?;
    service.run()
}
```

Avoid global mutable initialization and production `unwrap()` when setup can
return an ordinary error. Arrange declaration and ownership order so runtimes,
stores, and other dependencies outlive the objects that use them.

## Keep dependencies and tooling intentional

Add a dependency together with the code that needs it. Select only required
features, periodically remove unused direct dependencies, and avoid enabling a
large convenience feature set when a few explicit features suffice.

Formatting and static analysis should be reproducible project-wide:

```bash
cargo fmt --check
cargo check
cargo clippy --all-targets --all-features -- -D warnings
```

Treat new warnings as errors during review. Introduce tests and CI checks at a
scope appropriate to the behavior and risk being changed.
