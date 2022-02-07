# How to run
First we need to build the project
```
cargo build
```
Then we need to run the server. In the current state it has to be run as the superuser to be able to manage cgroups
# server
```
sudo ./target/debug/jobdaemon --key certs/server.key.der --cert certs/server.cert.der --ca-cert certs/ca.cert.der --addr 127.0.0.1:50050
```

Then we can execute client commands
# client
```
./target/debug/jobclient --key certs/admin.key.der --cert certs/admin.cert.der --ca-cert certs/ca.cert.der --addr https://localhost:50050 start sleep 60
```
