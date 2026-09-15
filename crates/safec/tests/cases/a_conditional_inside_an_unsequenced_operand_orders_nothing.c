void *malloc(int n);
void free(void *p);
int f(int w, int q) {
    int *p = malloc(4);
    int x = w + ((free(p), q) ? *p : 0);
    return x;
}
