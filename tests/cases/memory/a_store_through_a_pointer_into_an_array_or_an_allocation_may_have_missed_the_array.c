void *malloc(int n);
int c(void);
int main(void) {
    int x = 0;
    int *m[2];
    int **g = m + 1;
    int **h = malloc(8);
    if (!h) return 0;
    if (c()) g = h;
    *g = &x;
    int *r = m[1];
    return r == 0;
}
