void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    int x = (*p = 1) + (free(p), 0);
    return x;
}
