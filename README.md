# bash scripts for endlessh auto reporting to abuseIpDb

includes 15min IP cache, each individual IP is reported every 15min max

## requirements
* [endlessh](https://github.com/skeeto/endlessh)
* [curl](https://github.com/curl/curl)
* [jq](https://github.com/stedolan/jq) (optional, could be removed)

* [abuseipdb](abuseipdb.com) account, api token

## usage

1. move your ssh port to some safer port (between 1024 and 65535)
2. replace `<api-token>` with your abuseipdb api token in `report.sh`
3. run `./tarpitReporter.sh`

=> IPs from stuck SSH login attempts are reported and logged in `reportedIps.txt`


## Docker Usage

This project can be run in a Docker container. To do so, you'll need to have Docker installed on your machine.

Build the Docker image with the following command:

```bash
docker build -t endlessh-reporter .
```

To run the Docker container, use the following command, replacing `your_api_token` with your actual API token:

```bash
docker run -it --rm -e API_TOKEN=your_api_token -p 2222:22 endlessh-reporter
```
