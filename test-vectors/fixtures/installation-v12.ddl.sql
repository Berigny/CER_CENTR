-- Synthetic stand-in for an existing system-of-record schema.
-- Not a real CER schema: the point is that evidence/0 hashes whatever
-- definition the system of record already has.
CREATE TABLE installation (
  installation_id          VARCHAR(32) PRIMARY KEY,
  system_type              VARCHAR(32) NOT NULL,
  inverter_model           VARCHAR(64),
  battery_model            VARCHAR(64),
  installer_accreditation  VARCHAR(32) NOT NULL,
  commissioned_on          DATE,
  capacity_kw              NUMERIC(7,3),
  status                   VARCHAR(16) NOT NULL
);
