terraform {
  required_version = ">= 1.6"
  required_providers {
    ovh = {
      source  = "ovh/ovh"
      version = "~> 1.0"
    }
    openstack = {
      source  = "terraform-provider-openstack/openstack"
      version = "~> 2.0"
    }
  }
}

# OVH Public Cloud (OpenStack-based) — better Terraform story than the
# legacy VPS line. Create the project in the OVH manager first; set:
#   OVH_ENDPOINT=ovh-eu OVH_APPLICATION_KEY=... OVH_APPLICATION_SECRET=... OVH_CONSUMER_KEY=...
#   OS_AUTH_URL=https://auth.cloud.ovh.net/v3 (handled below)

variable "ovh_service_name"    { type = string  description = "OVH Public Cloud project ID (also called serviceName)." }
variable "hostname"             { type = string  default = "auditnetwork" }
variable "domain"               { type = string }
variable "container_image"      { type = string  default = "ghcr.io/me1iissa/auditnetwork:latest" }
variable "ssh_authorized_keys"  { type = list(string) }
variable "region"               { type = string  default = "GRA11" }  # Gravelines, FR. SBG, BHS, WAW also.
variable "flavor"               { type = string  default = "b3-8" }   # 2 vCPU / 8 GB / 50 GB local. d2-2 (~€5) is the cheapest.
variable "image_name"           { type = string  default = "Ubuntu 24.04" }

provider "ovh" {
  endpoint = "ovh-eu"
}

# OVH Public Cloud uses standard OpenStack. We generate a scoped OpenStack
# user under the OVH project and feed its credentials to the OpenStack
# provider below.
resource "ovh_cloud_project_user" "tf" {
  service_name = var.ovh_service_name
  description  = "terraform-${var.hostname}"
  role_names   = ["compute_operator", "network_operator", "image_operator", "objectstore_operator"]
}

provider "openstack" {
  auth_url    = "https://auth.cloud.ovh.net/v3"
  domain_name = "Default"
  user_name   = ovh_cloud_project_user.tf.username
  password    = ovh_cloud_project_user.tf.password
  tenant_name = var.ovh_service_name
  region      = var.region
}

resource "openstack_compute_keypair_v2" "deploy" {
  name       = "${var.hostname}-deploy"
  public_key = var.ssh_authorized_keys[0]
}

data "openstack_images_image_v2" "ubuntu" {
  name        = var.image_name
  most_recent = true
}

resource "openstack_networking_secgroup_v2" "web" {
  name        = "${var.hostname}-web"
  description = "ssh + http + https"
}
resource "openstack_networking_secgroup_rule_v2" "ssh" {
  direction        = "ingress"
  ethertype        = "IPv4"
  protocol         = "tcp"
  port_range_min   = 22
  port_range_max   = 22
  security_group_id = openstack_networking_secgroup_v2.web.id
}
resource "openstack_networking_secgroup_rule_v2" "http" {
  direction        = "ingress"
  ethertype        = "IPv4"
  protocol         = "tcp"
  port_range_min   = 80
  port_range_max   = 80
  security_group_id = openstack_networking_secgroup_v2.web.id
}
resource "openstack_networking_secgroup_rule_v2" "https" {
  direction        = "ingress"
  ethertype        = "IPv4"
  protocol         = "tcp"
  port_range_min   = 443
  port_range_max   = 443
  security_group_id = openstack_networking_secgroup_v2.web.id
}

resource "openstack_compute_instance_v2" "an" {
  name        = var.hostname
  flavor_name = var.flavor
  key_pair    = openstack_compute_keypair_v2.deploy.name
  image_id    = data.openstack_images_image_v2.ubuntu.id
  security_groups = [openstack_networking_secgroup_v2.web.name]
  network { name = "Ext-Net" }
  user_data = templatefile("${path.module}/../common/cloud-init.yaml.tftpl", {
    domain              = var.domain
    container_image     = var.container_image
    ssh_authorized_keys = var.ssh_authorized_keys
  })
}

output "ipv4_address" { value = openstack_compute_instance_v2.an.access_ip_v4 }
output "ssh"          { value = "ssh deploy@${openstack_compute_instance_v2.an.access_ip_v4}" }
output "dns_target"   { value = "Point ${var.domain} A record at ${openstack_compute_instance_v2.an.access_ip_v4}" }
