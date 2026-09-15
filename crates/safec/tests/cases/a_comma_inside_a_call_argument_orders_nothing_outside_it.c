void *malloc(int n);
void free(void *p);
int g(int a, int b);
int f(void) {
    int *p = malloc(4);
    int x = g((free(p), 0), *p);
    return x;
}
