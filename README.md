# k8scp-rust

A command line tool for copying files to K8s pods, developed in rust.

## Usage

```bash
k8scp-rust [OPTIONS] --kubeconfig <KUBECONFIG> --pod <POD> --src <SRC> --dst <DST>

Options:
-k, --kubeconfig <KUBECONFIG>  
-n, --namespace <NAMESPACE>    [default: default]
-p, --pod <POD>                
-c, --container <CONTAINER>    [default: ]
-s, --src <SRC>                
-d, --dst <DST>                
-h, --help                     Print help information
-V, --version                  Print version information
```