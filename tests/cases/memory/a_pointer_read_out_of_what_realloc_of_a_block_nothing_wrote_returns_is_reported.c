void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int main(void) {
    int **old = malloc(8);
    if (!old) return 1;
    int **nw = realloc(old, 8);
    if (!nw) { free(old); return 1; }
    int *p = *nw;
    int y = p != 0;
    free(nw);
    return y;
}
