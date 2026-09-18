void *malloc(int n);
void free(void *p);
int g(int a);
int f(void) {
    int *p = malloc(4);
    int *q = malloc(4);
    int x = g(*q) + (free(p), 0);
    return x;
}
