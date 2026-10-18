void *malloc(int n);
void free(void *p);
void g(int a, int b);
int f(void) {
    int *p = malloc(4);
    g(0, (free(p), 1) && *p);
    return 0;
}
