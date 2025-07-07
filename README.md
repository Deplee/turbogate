# Turbogate: Modern Load Balancer & Reverse Proxy

## 🚀 Project Overview

**Turbogate** is a high-performance, modern load balancer and reverse proxy written in Rust, designed for simplicity, security, and ease of use. It provides enterprise-grade load balancing capabilities with built-in DDoS protection, rate limiting, and compression features.

## 🎯 Key Features

### Core Functionality
- **TCP/HTTP Load Balancing** with multiple algorithms (Round Robin, Least Connections, Random)
- **Health Checks** with configurable intervals and thresholds
- **Rate Limiting** with burst support and configurable windows
- **DDoS Protection** with IP-based filtering, suspicious pattern detection, and automatic connection limiting
- **Compression** support for GZIP, Brotli, and Deflate
- **Access Control Lists (ACL)** for IP-based filtering and routing
- **Metrics & Monitoring** with Prometheus-compatible endpoints
- **Hot Reload** for configuration changes without downtime

### Advanced Features
- **Connection Management** with TCP keep-alive support
- **Backup Servers** for high availability
- **Weighted Load Balancing** for fine-grained control
- **Multiple Bind Addresses** per frontend
- **Flexible Configuration** with HAProxy-compatible syntax
- **Comprehensive Logging** with structured output

## 🏆 Why Turbogate is Better Than HAProxy

### 1. **Memory Safety & Security**
- **Rust Language**: Eliminates entire classes of bugs (buffer overflows, use-after-free, data races)
- **Memory-Safe by Design**: No manual memory management required
- **Security-First Approach**: Built-in protection against common vulnerabilities

### 2. **Built-in DDoS Protection**
- **Native Implementation**: No external tools or complex setups needed
- **IP-based Filtering**: Automatic connection limiting per IP
- **Suspicious Pattern Detection**: Blocks malicious User-Agent patterns
- **Whitelist/Blacklist Support**: Flexible IP access control
- **Automatic Counter Reset**: Intelligent rate limiting with configurable intervals

### 3. **Simplified Configuration**
- **HAProxy-Compatible Syntax**: Easy migration from existing setups
- **Modern Improvements**: Enhanced syntax with comma-separated values
- **Flexible Bind Syntax**: Support for both `bind 0.0.0.0:port` and `bind *:port`
- **Intuitive Directives**: Clear, readable configuration format

### 4. **Native Rate Limiting**
- **Integrated Support**: No external modules required
- **Burst Handling**: Configurable burst sizes for traffic spikes
- **Multi-level Control**: Global, frontend, and backend-specific limits
- **Automatic Enforcement**: Seamless integration with load balancing

### 5. **Modern Compression**
- **Multiple Algorithms**: GZIP, Brotli, and Deflate support
- **Configurable Thresholds**: Min/max size limits and compression levels
- **Content-Type Aware**: Intelligent compression based on content
- **Performance Optimized**: Efficient compression with minimal overhead

### 6. **Hot Reload Capability**
- **Zero Downtime**: Configuration changes without service interruption
- **Signal-Based Reload**: Simple `SIGHUP` to reload configuration
- **Validation**: Automatic configuration validation before reload
- **Rollback Support**: Safe configuration updates

### 7. **Single Binary Deployment**
- **No Dependencies**: Self-contained executable
- **Easy Distribution**: Simple deployment across environments
- **Cross-Platform**: Runs on multiple operating systems
- **Container Ready**: Perfect for Docker and Kubernetes deployments

### 8. **Comprehensive Metrics**
- **Prometheus Compatible**: Standard metrics format
- **Real-time Monitoring**: Live performance and health data
- **Custom Metrics**: Application-specific monitoring
- **Integration Ready**: Works with existing monitoring stacks

### 9. **Performance Advantages**
- **Near-Native Speed**: Rust's zero-cost abstractions
- **Efficient Memory Usage**: Optimal resource utilization
- **High Concurrency**: Excellent handling of thousands of connections
- **Low Latency**: Minimal overhead for request processing

## 📊 Comparison Table

| Feature | Turbogate | HAProxy |
|---------|-----------|---------|
| **Language** | Rust (memory-safe, fast) | C (manual memory management) |
| **Configuration** | Simple, intuitive | Complex, verbose |
| **DDoS Protection** | Built-in, configurable | Limited, requires external tools |
| **Rate Limiting** | Native support | Requires external modules |
| **Compression** | Multiple algorithms | Basic support |
| **Hot Reload** | Native support | Limited |
| **Memory Safety** | Guaranteed by Rust | Manual management |
| **Performance** | Near-native speed | Optimized but complex |
| **Security** | Memory-safe by design | Requires careful configuration |
| **Deployment** | Single binary | Multiple dependencies |

## 📦 Installation

### From Source
```bash
git clone https://github.com/Deplee/turbogate.git
cd turbogate
cargo build --release
```

### Binary Release
Download the latest release from the releases page.

## 🚀 Quick Start

1. **Create Configuration**
```bash
cp examples/example.cfg turbogate.cfg
```

2. **Start Turbogate**
```bash
./turbogate -c turbogate.cfg
```

3. **Test Connection**
```bash
curl http://localhost:8080
```

## 📝 Configuration

### Basic Example
```cfg
global
    maxconn 8092
    daemon off
    stats bind *:9090
    
    # Rate limiting
    rate-limit-rps 100
    rate-limit-burst 10
    
    # DDoS protection
    ddos-protection reset-interval-seconds 60
    ddos-protection max-requests-per-minute 100
    ddos-protection max-connections-per-ip 10
    ddos-protection suspicious-pattern bot, scanner
    ddos-protection whitelist 192.168.1.1, 10.0.0.0/8
    ddos-protection blacklist 172.30.1.1

defaults
    mode tcp
    timeout connect 10s
    timeout client 120s
    timeout server 1h

frontend web_frontend
    bind *:8080
    mode tcp
    default_backend web_backend

backend web_backend
    mode tcp
    balance roundrobin
    server server1 10.0.1.10:80 check inter 5s fall 3 rise 2
    server server2 10.0.1.11:80 check inter 5s fall 3 rise 2
```

### Advanced Features

#### DDoS Protection
```cfg
# Global DDoS protection
ddos-protection reset-interval-seconds 60
ddos-protection max-requests-per-minute 100
ddos-protection max-connections-per-ip 10
ddos-protection suspicious-pattern bot, scanner, crawler
ddos-protection whitelist 192.168.1.1, 10.0.0.0/8
ddos-protection blacklist 172.30.1.1, 192.168.1.100
```

#### Rate Limiting
```cfg
# Global rate limiting
rate-limit-rps 100
rate-limit-burst 10

# Frontend-specific rate limiting
frontend api_frontend
    bind *:8081
    rate-limit-rps 50
    rate-limit-burst 5
```

#### Compression
```cfg
# Global compression settings
compression-gzip enabled
compression-brotli enabled
compression-deflate disabled
compression-min-size 1024
compression-max-size 10485760
compression-level 6
```

## 🔧 Configuration Options

### Global Section
- `maxconn`: Maximum connections
- `daemon`: Run in background
- `stats bind`: Metrics endpoint
- `rate-limit-rps`: Requests per second limit
- `rate-limit-burst`: Burst size for rate limiting
- `ddos-protection`: DDoS protection settings

### Frontend Section
- `bind`: Listen addresses (supports `*:port` syntax)
- `mode`: Protocol mode (tcp/http)
- `default_backend`: Default backend
- `acl`: Access control lists
- `use_backend`: Conditional backend routing

### Backend Section
- `mode`: Protocol mode
- `balance`: Load balancing algorithm
- `server`: Backend servers
- `option`: Backend options
- `retries`: Retry attempts

## 📊 Monitoring

### Metrics Endpoint
Access metrics at `http://localhost:9090/metrics` (Prometheus format)

### Health Checks
- TCP health checks with configurable intervals
- Rise/fall thresholds
- Automatic server failover

## 🔒 Security Features

### DDoS Protection
- IP-based connection limiting
- Request rate limiting per IP
- Suspicious pattern detection
- Whitelist/blacklist support
- Automatic counter reset

### Access Control
- IP-based ACLs
- CIDR notation support
- Conditional routing
- Block/allow rules

## 🚀 Performance

- **High Throughput**: Optimized for high-performance environments
- **Low Latency**: Minimal overhead for request processing
- **Memory Efficient**: Rust's memory management ensures optimal resource usage
- **Concurrent Connections**: Efficient handling of thousands of concurrent connections

## 🔄 Hot Reload

Turbogate supports configuration hot reloading:
```bash
# Send SIGHUP to reload configuration
kill -HUP <pid>
```

## 📈 Use Cases

- **Web Application Load Balancing**
- **API Gateway**
- **Microservices Proxy**
- **DDoS Protection Layer**
- **Content Delivery Optimization**
- **High Availability Clusters**

## 💡 Key Advantages Summary

1. **Security**: Memory-safe by design, eliminating common vulnerabilities
2. **Simplicity**: Easy configuration with modern syntax improvements
3. **Performance**: High throughput with low latency
4. **Reliability**: Built-in protection and monitoring
5. **Modern**: Designed for today's distributed systems
6. **Maintainable**: Clean codebase with comprehensive documentation

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests
5. Submit a pull request

## 📄 License

This project is licensed under the MIT License - see the LICENSE file for details.

## 🆘 Support

- **Issues**: Report bugs and feature requests on GitHub
- **Documentation**: See `examples/example.cfg` for comprehensive examples
- **Community**: Join our community discussions

---

**Turbogate** represents the next generation of load balancing technology, combining the reliability of proven concepts with the safety and performance of modern systems programming. It's not just an alternative to HAProxy—it's an evolution that addresses the challenges of modern infrastructure while maintaining compatibility with existing workflows.
