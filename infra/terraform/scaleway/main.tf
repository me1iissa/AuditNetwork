terraform {
  required_version = ">= 1.6"
  required_providers {
    scaleway = {
      source  = "scaleway/scaleway"
      version = "~> 2.41"
    }
  }
}

# Auth via env vars (recommended):
#   export SCW_ACCESS_KEY=...
#   export SCW_SECRET_KEY=...
#   export SCW_DEFAULT_PROJECT_ID=...
#   export SCW_DEFAULT_ORGANIZATION_ID=...
# Or via ~/.config/scw/config.yaml.

variable "hostname"            { type = string  default = "auditnetwork" }
variable "domain"              { type = string }
variable "container_image"     { type = string  default = "ghcr.io/me1iissa/auditnetwork:latest" }
variable "ssh_authorized_keys" { type = list(string) }
variable "zone"                { type = string  default = "fr-par-1" }    # fr-par-{1,2,3} / nl-ams-{1,2,3} / pl-waw-{1,2}
variable "instance_type"       { type = string  default = "PLAY2-NANO" }  # PLAY2-NANO (1c/2G ~€5) / PLAY2-MICRO (1c/4G ~€10) / COPARM1-2C-8G (ARM)
variable "image"               { type = string  default = "ubuntu_jammy" }

provider "scaleway" {
  zone = var.zone
}

resource "scaleway_iam_ssh_key" "deploy" {
  for_each   = { for i, k in var.ssh_authorized_keys : i => k }
  name       = "${var.hostname}-${each.key}"
  public_key = each.value
}

resource "scaleway_instance_security_group" "web" {
  name                    = "${var.hostname}-web"
  inbound_default_policy  = "drop"
  outbound_default_policy = "accept"
  inbound_rule { action = "accept" port = 22  ip_range = "0.0.0.0/0" }
  inbound_rule { action = "accept" port = 80  ip_range = "0.0.0.0/0" }
  inbound_rule { action = "accept" port = 443 ip_range = "0.0.0.0/0" }
}

resource "scaleway_instance_ip" "an" {}

resource "scaleway_instance_server" "an" {
  name       = var.hostname
  type       = var.instance_type
  image      = var.image
  ip_id      = scaleway_instance_ip.an.id
  security_group_id = scaleway_instance_security_group.web.id
  user_data = {
    cloud-init = templatefile("${path.module}/../common/cloud-init.yaml.tftpl", {
      domain              = var.domain
      container_image     = var.container_image
      ssh_authorized_keys = var.ssh_authorized_keys
    })
  }
  tags = ["app=auditnetwork", "env=prod"]
  # Scaleway has no separate "create the keypair" step at the API level for
  # cloud-init users; the cloud-init template injects keys on the deploy user.
  depends_on = [scaleway_iam_ssh_key.deploy]
}

output "ipv4_address" { value = scaleway_instance_ip.an.address }
output "ssh"          { value = "ssh deploy@${scaleway_instance_ip.an.address}" }
output "dns_target"   { value = "Point ${var.domain} A record at ${scaleway_instance_ip.an.address}" }
