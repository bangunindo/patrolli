-- UUIDv7 for the statements that mint ids in SQL. PostgreSQL has a builtin
-- uuidv7() only from 18 on; this one needs nothing newer than gen_random_uuid()
-- (core since 13). It takes a random v4, writes the 48-bit unix millisecond
-- timestamp over its first six bytes and turns the version nibble from 4 into 7.
-- Unlike the builtin (and Uuid::now_v7) it is not monotonic within one
-- millisecond: ids minted in the same millisecond sort in random order.
CREATE FUNCTION patrolli_uuidv7() RETURNS uuid
    LANGUAGE sql
    VOLATILE
    PARALLEL SAFE
AS
$$
SELECT encode(
               set_bit(
                       set_bit(
                               overlay(uuid_send(gen_random_uuid())
                                       placing substring(int8send(floor(extract(epoch from clock_timestamp()) * 1000)::bigint) from 3)
                                       from 1 for 6),
                               52, 1),
                       53, 1),
               'hex')::uuid;
$$;
