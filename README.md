# rindexer

inspiration - https://ponder.sh/docs/guides/add-contracts

networks - https://github.com/ponder-sh/ponder/blob/83e2b4a7a05d847832ba60adde361736deeb3b2c/packages/core/src/config/networks.ts#L22

eth_getLogs - https://github.com/ponder-sh/ponder/blob/83e2b4a7a05d847832ba60adde361736deeb3b2c/packages/core/src/sync-historical/service.ts#L946

checklist v1.0:

- csv autogenerate code
- internal tables in postgres like last seen blocks etc
- finish command line https://ponder.sh/docs/api-reference/create-ponder
- look into load balancing of RPCs
- investigate graphql API exposing
- look into https://diesel.rs/ for mapping schemas etc
- investigate indexing contracts that are deployed within an event onchain 
- investigate handle reorgs process
  - add ability to config distance from blocks
- handle other providers block ranges in start indexing
- docs
- look into deployments to make it easy to do

future features:
- other db support
- look into internal caching to make things faster
- look into dependency mappings to allow you to index based on trees structure
- POC with shadow events using foundry as you index

// cargo new node --lib

// cargo run -- start help

Flows:

- User creates a new yaml file and maps config
- User runs rindexer generate which generates the mappings files types
- User then uses that generated file to register mappings with the indexer
