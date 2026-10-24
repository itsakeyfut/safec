void *malloc(int n);
void free(void *p);
int k(int a);
int f(void) {
    int *p = malloc(4);
    int x = k(0) + ((free(p), 1) && *p);
    return x;
}
