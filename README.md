# oneway
Small utility to transfer files over a readonly/writeonly network

## Build
Just run
```bash
cargo build --examples
```

## Server
Both clients and server uses a .ini style configuratin file being passed as their first and only argument.

The server wait for new requests for any client and will create and update files according to the clients specifications.

## Client
Sends a bunch of files specified from the configuration file

## Config
```dosini
; Maximum size of chunks being sent to the server. The server use this key to get a hint on buffers preallocation
mtu = 2048

; Timeout on receivin frames from client. Not used anymore
recv_timeout =  1

; Address to bind for the server of to connect to for the clients
address = 127.0.0.1:12345

; Root directory to search for files for the client or where to store files for the server
root = data/

; Number of time to send a chunk of data
remission_count = 3
```
