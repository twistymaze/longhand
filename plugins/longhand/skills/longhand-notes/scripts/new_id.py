# /// script
# requires-python = ">=3.9"
# dependencies = []
# ///
"""Print a new id for a summary prompt file.

    python3 new_id.py

The id is a random UUID, in the format that Longhand writes. Write it as
the `id` of each new prompt file. A copy of a prompt file must get a new
id, or Longhand shows the two files as one prompt with a conflict.

The script reads and writes no file.
"""

import sys
import uuid

sys.dont_write_bytecode = True


def new_id():
    return str(uuid.uuid4())


def main():
    sys.stdout.write(new_id() + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
