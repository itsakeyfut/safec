void *malloc(int n);
void free(void *p);
int g(int a);
int f(int w) {
    int *p = malloc(4);
    if (w) { p = malloc(8); }
    int x = g(*p) + (free(p), 0);
    return x;
}
