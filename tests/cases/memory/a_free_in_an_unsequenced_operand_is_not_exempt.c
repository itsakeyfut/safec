void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    free(p);
    int x = (p = 0, 1) + (free(p), 0);
    return x;
}
