void *realloc(void *p, int n);
void free(void *p);
int main(void) {
    int **old = 0;
    int **pp = realloc(old, 8);
    if (!pp) return 1;
    int *p = *pp;
    free(pp);
    return p == 0;
}
