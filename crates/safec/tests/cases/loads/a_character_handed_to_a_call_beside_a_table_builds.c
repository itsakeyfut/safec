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
    *p = 1;
    *tab = p;
    char *s = malloc(4);
    if (s == 0) {
        return 0;
    }
    *s = 65;
    char c = *s;
    emit(c);
    int r = **tab;
    free(p);
    free(tab);
    free(s);
    return r;
}
