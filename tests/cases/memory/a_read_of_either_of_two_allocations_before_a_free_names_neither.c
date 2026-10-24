void *malloc(int n);
void free(void *p);
int g(int a);
int f(int w) {
    int *a = malloc(4);
    int *b = malloc(8);
    int *p;
    if (w) { p = a; } else { p = b; }
    int x = g(*p) + (free(p), 0);
    return x;
}
