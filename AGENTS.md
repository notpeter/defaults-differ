# Repository Instructions

- When help output changes, update the README Help section with:

````sh
HELP="$(cargo run -q -- --help)" awk 'BEGIN{b="## Help\n\n```text\n"ENVIRON["HELP"]"\n```\n";s=0;w=0} $0=="## Help"{s=1;next} s&&$0=="## Usage"{print b;print;s=0;w=1;next} s{next} $0=="## Usage"&&!w{print b;w=1} {print}' README.md > README.md.tmp && mv README.md.tmp README.md
````
