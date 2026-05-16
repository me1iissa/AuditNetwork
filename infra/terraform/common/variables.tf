# Shared input variables. Each provider folder re-declares these so it
# can be applied standalone; values are passed in via terraform.tfvars
# or -var/-var-file at apply time.

variable "hostname" {
  description = "Hostname for the box (also used as the Terraform display name)."
  type        = string
  default     = "auditnetwork"
}

variable "domain" {
  description = "Fully-qualified DNS name; used by Caddy for TLS via Let's Encrypt."
  type        = string
}

variable "container_image" {
  description = "Container image tag to deploy."
  type        = string
  default     = "ghcr.io/me1iissa/auditnetwork:latest"
}

variable "ssh_authorized_keys" {
  description = "SSH public keys authorised on the `deploy` user."
  type        = list(string)
}
