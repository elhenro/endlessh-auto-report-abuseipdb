FROM ubuntu:mantic-20240405

RUN apt-get update && apt-get install -y \
    endlessh \
    curl \
    jq \
    git

RUN git clone https://github.com/arbs09/endlessh-auto-report-abuseipdb.git

WORKDIR /endlessh-auto-report-abuseipdb

RUN sed -i 's/<api-token>/${API_TOKEN}/g' report.sh

RUN chmod +x tarpitReporter.sh report.sh cache.sh

CMD ["bash", "./tarpitReporter.sh"]
