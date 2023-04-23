#!/bin/bash
# Place the data directory inside the project directory
PGDATA="$(pwd)/postgres"
export PGDATA
# Place Postgres' Unix socket inside the data directory
export PGHOST="$PGDATA"
export PGUSER="adsbx"

if [[ ! -d "$PGDATA" ]]; then
	# If the data directory doesn't exist, create an empty one, and...
	initdb
	# ...configure it to listen only on the Unix socket, and...
	cat >>"$PGDATA/postgresql.conf" <<-EOF
		listen_addresses = 'localhost'
		unix_socket_directories = '$PGHOST'
	EOF
	# ...create a database using the name Postgres defaults to.
	echo "CREATE DATABASE $PGUSER;" | postgres --single -E postgres
	# Create a user with the same name as the database.
	echo "CREATE USER $PGUSER WITH PASSWORD '$PGUSER';" | postgres --single -E postgres
	# Now grant all privileges to the user we'll be using
	echo "GRANT ALL PRIVILEGES ON DATABASE $PGUSER TO $PGUSER;" | postgres --single -E postgres
	# Make the user a superuser.
	# echo "ALTER USER $PGUSER WITH SUPERUSER;" | postgres --single -E postgres
fi
