void *malloc(int n);
void free(void *p);
void emit(char c);

int main(void) {
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    *p = 3;
    *tab = p;
    char *src = malloc(8);
    if (src == 0) {
        return 0;
    }
    char *dst = malloc(8);
    if (dst == 0) {
        return 0;
    }
    *src = 0;
    char *a = src;
    char *b = dst;
    char c = *a;
    while (c != 0) {
        if (b != 0) {
            *b = c;
        }
        emit(c);
        a = a + 1;
        b = b + 1;
        c = 0;
        if (a != 0) {
            c = *a;
        }
    }
    int r = **tab;
    free(p);
    free(tab);
    free(src);
    free(dst);
    return r;
}
