void *malloc(int n);
void free(void *p);
int f(int c) {
    int *a = malloc(4);
    int *b = malloc(4);
    free(a);
    int *r = c ? a : b;
    int x = *r + (free(b), 0);
    return x;
}
