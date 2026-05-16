terraform {
  required_version = ">= 1.6"
  required_providers {
    hcloud = {
      source  = "hetznercloud/hcloud"
      version = "~> 1.48"
    }
  }
}

variable "hcloud_token" {
  description = "Hetzner Cloud API token (read+write). Provide via HCLOUD_TOKEN env or TF_VAR_hcloud_token."
  type        = string
  sensitive   = true
}

variable "hostname"            { type = string  default = "auditnetwork" }
variable "domain"              { type = string }
variable "container_image"     { type = string  default = "ghcr.io/me1iissa/auditnetwork:latest" }
variable "ssh_authorized_keys" { type = list(string) }
variable "location"            { type = string  default = "fsn1" }   # Falkenstein. nbg1, hel1, ash, hil, sin also work.
variable "server_type"         { type = string  default = "cax11" }  # 4 ARM / 8 GB / 40 GB ~€3.79/mo. Use cx22 for x86.
variable "image"               { type = string  default = "ubuntu-24.04" }

provider "hcloud" {
  token = var.hcloud_token
}

resource "hcloud_ssh_key" "deploy" {
  for_each   = { for i, k in var.ssh_authorized_keys : i => k }
  name       = "${var.hostname}-${each.key}"
  public_key = each.value
}

resource "hcloud_firewall" "web" {
  name = "${var.hostname}-fw"
  rule {
    direction = "in"
    protocol  = "tcp"
    port      = "22"
    source_ips = ["0.0.0.0/0", "::/0"]
  }
  rule {
    direction = "in"
    protocol  = "tcp"
    port      = "80"
    source_ips = ["0.0.0.0/0", "::/0"]
  }
  rule {
    direction = "in"
    protocol  = "tcp"
    port      = "443"
    source_ips = ["0.0.0.0/0", "::/0"]
  }
}

resource "hcloud_server" "an" {
  name         = var.hostname
  server_type  = var.server_type
  image        = var.image
  location     = var.location
  ssh_keys     = [for k in hcloud_ssh_key.deploy : k.id]
  firewall_ids = [hcloud_firewall.web.id]
  user_data = templatefile("${path.module}/../common/cloud-init.yaml.tftpl", {
    domain              = var.domain
    container_image     = var.container_image
    ssh_authorized_keys = var.ssh_authorized_keys
  })
  labels = {
    app  = "auditnetwork"
    env  = "prod"
  }
}

output "ipv4_address" { value = hcloud_server.an.ipv4_address }
output "ipv6_address" { value = hcloud_server.an.ipv6_address }
output "ssh"          { value = "ssh deploy@${hcloud_server.an.ipv4_address}" }
output "dns_target"   { value = "Point ${var.domain} A record at ${hcloud_server.an.ipv4_address}" }
