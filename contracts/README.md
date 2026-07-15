# Public contracts

OpenAPI 3.1 and JSON Schema 2020-12 documents in this directory are the
canonical wire definitions. The current files establish only health,
capability, and error envelopes for the private alpha; data operations are
added contract-first in Phase 5.

Generated Rust, TypeScript, and Python models must be reproducible from these
documents, and generated-code drift is a CI failure.
