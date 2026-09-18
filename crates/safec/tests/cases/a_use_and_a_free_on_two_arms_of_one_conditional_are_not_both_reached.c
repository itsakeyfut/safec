void *malloc(int n);
void free(void *p);
int f(int w) {
    int *p = malloc(4);
    int x = (w ? *p : (free(p), 0)) + 1;
    return x;
}
