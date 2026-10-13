"""The tests of the longhand-notes scripts. They read the shared test
storage folder `testdata/storage-folder/` (KTD13)."""

import sys

# The tests import the scripts. The scripts must not leave bytecode next
# to them, also in a test run.
sys.dont_write_bytecode = True
