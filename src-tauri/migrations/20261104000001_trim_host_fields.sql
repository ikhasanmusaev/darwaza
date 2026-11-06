-- Пробелы по краям адреса, имени пользователя и названия хоста.
--
-- Хост, сохранённый как " 1.1.1.1", давал соединение на «root@ 1.1.1.1», и оно
-- не работало. Новые записи обрезаются при сохранении; здесь чистим уже
-- лежащие в базе, включая импортированные и пришедшие с синхронизацией.
UPDATE hosts
   SET name = TRIM(name),
       hostname = TRIM(hostname),
       username = TRIM(username)
 WHERE name != TRIM(name)
    OR hostname != TRIM(hostname)
    OR username != TRIM(username);
