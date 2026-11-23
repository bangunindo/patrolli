-- UUIDv7 for the statements that mint ids in SQL. PostgreSQL has a builtin
-- uuidv7() only from 18 on, so this picks at call time: the builtin where the
-- server has it, otherwise a fallback that needs nothing newer than
-- gen_random_uuid() (core since 13). Choosing per call rather than when the
-- migration runs means a server upgraded later starts using the builtin by
-- itself. It has to be plpgsql: its statements are only planned when they
-- first run, so the uuidv7() reference never reaches the parser of an older server.
--
-- The fallback takes a random v4, writes the 48-bit unix millisecond timestamp
-- over its first six bytes and turns the version nibble from 4 into 7. Unlike
-- the builtin (and Uuid::now_v7) it is not monotonic within one millisecond:
-- ids minted in the same millisecond sort in random order.
CREATE FUNCTION patrolli_uuidv7() RETURNS uuid
    LANGUAGE plpgsql
    VOLATILE
    PARALLEL SAFE
AS
$$
BEGIN
    IF current_setting('server_version_num')::int >= 180000 THEN
        RETURN uuidv7();
    END IF;
    RETURN encode(
            set_bit(
                    set_bit(
                            overlay(uuid_send(gen_random_uuid())
                                    placing substring(int8send(floor(extract(epoch from clock_timestamp()) * 1000)::bigint) from 3)
                                    from 1 for 6),
                            52, 1),
                    53, 1),
            'hex')::uuid;
END
$$;
