endlessh -f endlessh-config-example.conf -v | \
  grep 'ACCEPT' --line-buffered | \
  while IFS= read -r line;
   do ./report.sh "$line";
  done
