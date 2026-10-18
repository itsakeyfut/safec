void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    int x = -(free(p), *p);
    return x;
}
