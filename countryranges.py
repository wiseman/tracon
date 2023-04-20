import json
import re
import sys


# Compile the regular expression
pattern = re.compile(
    r'{ start: (0x[0-9A-F]+), end: (0x[0-9A-F]+), country: "(.+?)", flag_image: "(.+?)" }'
)


def process_file(path: str):

    # Initialize an empty list to store the data
    data = []

    # Read the lines of JavaScript code from the input file
    with open(path) as f:
        lines = f.readlines()

    # Extract the data from the lines of code
    for line in lines:
        match = pattern.search(line)
        if not match:
            continue
        start = match.group(1)
        end = match.group(2)
        country = match.group(3)
        data.append(
            {"start": start, "end": end, "country": country, "priority": len(data) + 1}
        )

    json.dump(data, sys.stdout, indent=2)


if __name__ == "__main__":
    process_file(sys.argv[1])
