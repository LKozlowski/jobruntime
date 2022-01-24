# Job runtime
This project provides an API to run arbitrary Linux processes. It consists of two programs, a client and a server.
A client is responsible for parsing arguments from the user and passing them to the server.
A server is responsible for processing users' requests and executing processes.
These applications will communicate with each other over a secure connection using TLS and utilize gRPC with protocol buffers to serialize and transport requests and responses.
A user can start a process, stop a process, query for process status, or get a streaming logs response.

# Architecture
```
            +-----------------+
            │    CLI client   │
            │  (gRPC client)  │ 
            +-----------------+
                    ▲
                    │ proto requests & responses over mTLS
                    ▼
            +-------------------+
            │      server       │
            │  (gRPC service)   │
            │  (worker library) │
            +-------------------+

```
## client
A CLI client is a simple program that will use protocol buffer messages to communicate with the server over gRPC.

It will be named `jobc` and its command will match the gRPC interface.

Example invocations of the CLI to show the CLI UX:

```
$ jobc start sleep 60
a62dfccc-2ff9-411d-a9ef-d4812ed3d867
```

```
$ jobc stop a62dfccc-2ff9-411d-a9ef-d4812ed3d867
```

```
$ jobc logs a62dfccc-2ff9-411d-a9ef-d4812ed3d867
"{ "log": "log message here", "stream": "stdout", "time": "2022-01-24T10:00:00.0Z }"
"{ "log": "another log message", "stream": "stdout", "time": "2022-01-24T10:01:00.0Z }"
<stream of log messages>
```

```
$ jobc status a62dfccc-2ff9-411d-a9ef-d4812ed3d867
running
```

```
$ jobc status a62dfccc-2ff9-411d-a9ef-d4812ed3d867
stopped (signal: 9)
```

```
$ jobc status a62dfccc-2ff9-411d-a9ef-d4812ed3d867
finished (exit code: 0)
```


## server
A server is an application that will expose a gRPC service and utilize a worker library to do all the heavy work. It's a thin layer over the worker library that handles authentication and communication with the client. I'll assume that the user starting the server application will have all permissions to manage cgroups and to create new processes.

## worker library
Its responsibilities are to create and stop a process, manage cgroups and resource limits, keep track of all process statuses and store and provide all the logs.

## API & CLI interface
CLI client will interact with the server over gRPC API. 
See [service.proto](/proto/service.proto)


## log streaming

Two main things need to be considered when implementing this feature.
We need to store all logs that the process produces so we are able to provide them whenever we ask for them. 
We need to stream new log messages after we provided all previous ones.

The server will create an `mpsc` channel and it will keep the receiving part of it. Each new process will get the sending part of the channel to send log messages back. It will simplify the part of storing all the logs in one place. 

The streaming part is a lot harder to do. There are a couple of options that we can consider.

We could create a new file for each process with the name of the job id and write all the messages here. Then we would read the already stored logs when a client connects and subscribe that client for file changes so when the new data is appended we would send it back to the client.
Since this is a toy project we don't want to clutter your environment with new files then we'll store all the data in memory.

The solution for this is as follows:
The server will keep all the logs in the hash map with the pairs `job_id: log buffer`. Then when the client requests logs, we will stream already existing logs back to the client and subscribe client for the new updates to that data structure.
One option is to utilize something like the `async_stream` crate. Then we could put that hash map into `RwLock` and then put that into `Arc` to share between clients. And then create async stream out of that. In this case, we'll need to request a lock for each write and read.
The other option is to create channels for each client, create another hash map for connected peers that would map peers to its channel, and just send messages through that channel. In this case, we don't need to lock that data structure as there will be only one place that manipulate that hash map.



## cgroups resource control
There are two versions of cgroups available, but this project will support only cgroups v2. It's a newer version with a simplified configuration.
It has only a single hierarchy which forms a tree structure. 

The idea to implement resource control in our project is as follows:

1) create a cgroup for the server process
2) enable `cpu`, `io`, `memory` controllers for the server cgroup
3) migrate server process into newly created cgroup

for each new job:

4) create a cgroup inside the server's cgroup directory using job_id as a name
5) set resource limits by updating values in the job's cgroup directory
6) spawn a new process
7) move the new process into the job's cgroup
8) execute a command 

There are a lot of values that can be set for each controller, but I've picked a few easiest to implement to cut the scope of the project.

# Authentication using mTLS
Secure communication is an important topic, so I will utilize the latest version of the Transport Layer Security (TLS 1.3) and the X.509 certificate.
Mutual TLS (mTLS) means that both parties at each end of the network have to validate each other certificate. It also ensures that certificates are valid and that both parties are who they claim to be.
To simplify the project, I'll provide several pre-generated certificates, both for a client and a server. These certificates will be valid forever. However, in real projects, we should use expiration dates on certificates.

# Authorization
As a simple authorization scheme, we can use role-based access control (RBAC).
I see two roles that we could implement:
- admin
- user

That would have these permissions:
| operation on jobs     | admin   | user  |
| --------------------- | ------- | ----- |
| start                 | yes     | yes   |
| stop own              | yes     | yes   |
| query own             | yes     | yes   |
| fetch logs own        | yes     | yes   |
| stop any              | yes     | no    |
| query any             | yes     | no    |
| fetch logs any        | yes     | no    |

The idea is that each job will have an associated owner, and an admin has the superpower to do anything with any job, whereas a user can only interact with its jobs. 

To store information about the user we can utilize X.509 certificate attributes and put the username into `CN - Common Name` and role(s) information into the `O - Organization` attribute.
I'll assume that our `certificate authority` creates only valid certificates with unique usernames and valid roles.

*If that's too much for this project, I can drop the role attribute and only leave the username attribute.*

