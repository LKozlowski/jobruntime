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

## server
A server is an application that will expose a gRPC service and utilize a worker library to do all the heavy work. It's a thin layer over the worker library that handles authentication and communication with the client. I'll assume that the user starting the server application will have all permissions to manage cgroups and to create new processes.

## worker library
Its responsibilities are to create and stop a process, manage cgroups and resource limits, keep track of all process statuses and store and provide all the logs.

## API & CLI interface
CLI client will interact with the server over gRPC API. 
See [service.proto](/proto/service.proto)

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
