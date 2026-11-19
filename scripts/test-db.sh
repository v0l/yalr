#!/usr/bin/env bash
set -euo pipefail

name="${YALR_TEST_DB_NAME:-yalr-test-db}"
port="${YALR_TEST_DB_PORT:-3307}"

if docker inspect "$name" >/dev/null 2>&1; then
    docker start "$name" >/dev/null
else
    docker run -d --name "$name" \
        --tmpfs /var/lib/mysql \
        -e MARIADB_ROOT_PASSWORD=root \
        -p "127.0.0.1:${port}:3306" \
        mariadb:11 \
        --innodb-flush-log-at-trx-commit=0 \
        --innodb-doublewrite=0 \
        --skip-log-bin \
        --sync-binlog=0 \
        --innodb-buffer-pool-size=1G \
        --max-connections=1000 >/dev/null
fi

port="$(docker port "$name" 3306/tcp | head -n1 | sed 's/.*://')"
for _ in $(seq 1 60); do
    if docker exec "$name" mariadb -uroot -proot -e 'SELECT 1' >/dev/null 2>&1; then
        echo "export YALR_TEST_DATABASE_URL=mysql://root:root@127.0.0.1:${port}"
        exit 0
    fi
    sleep 1
done
echo "MariaDB in $name did not become ready" >&2
exit 1
