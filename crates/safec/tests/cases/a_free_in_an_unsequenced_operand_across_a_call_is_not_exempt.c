void *malloc(int n);
void free(void *p);
int g(int n);
int f(void) {
    int *p = malloc(4);
    free(p);
    int x = (p = 0, 1) + (g(0) + (free(p), 0));
    return x;
}
