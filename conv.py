import sys
import pyarrow as pa
import pyarrow.parquet as pq
import json
import bz2

def json_to_arrow_schema(json_obj):
    """
    Converts a Python JSON object into a PyArrow schema.

    Args:
        json_obj: A Python JSON object.

    Returns:
        A PyArrow schema.
    """
    fields = {}
    for key, value in json_obj.items():
        if isinstance(value, dict):
            # Recursively convert sub-objects to PyArrow structs
            fields[key] = pa.field(key, pa.struct(json_to_arrow_schema(value)))
        elif isinstance(value, list):
            if value:
                # Get the data type of the list items
                item_type = json_to_arrow_schema(value[0])
            else:
                # Use string as the default item type for empty lists
                item_type = pa.string()
            fields[key] = pa.field(key, pa.list_(item_type))
        elif isinstance(value, bool):
            fields[key] = pa.field(key, pa.bool_())
        elif isinstance(value, (int, float)):
            fields[key] = pa.field(key, pa.float64())
        else:
            fields[key] = pa.field(key, pa.string())
    return pa.schema(fields.values())

# Get the list of JSON file paths from the command line arguments
json_files = sys.argv[1:]

tables = []
for json_file in json_files:
    # Check if the file is bzip2 compressed
    is_bz2 = json_file.endswith('.bz2')

    # Open the file for reading, possibly as a bz2 file
    if is_bz2:
        f = bz2.BZ2File(json_file, 'r')
    else:
        f = open(json_file, 'r')

    # Load the JSON data
    with f:
        data = json.load(f)
    
    # Determine the PyArrow schema from the JSON data
    schema = json_to_arrow_schema(data)
    
    # Convert the JSON data to a PyArrow table
    table = pa.Table.from_pydict(data, schema=schema)
    
    tables.append(table)

# Concatenate the tables into a single table
table = pa.concat_tables(tables)

# Write the PyArrow table to a parquet file
parquet_file = "output.parquet"
pq.write_table(table, parquet_file)

print(f"Converted {len(json_files)} JSON files to {parquet_file}")
