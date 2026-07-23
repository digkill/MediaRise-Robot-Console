-- Векторные эмбеддинги для семантического поиска по памяти робота.
-- Хранится как little-endian f32 массив (text-embedding-3-small, 1536 float).

ALTER TABLE device_memory ADD COLUMN embedding MEDIUMBLOB NULL;
