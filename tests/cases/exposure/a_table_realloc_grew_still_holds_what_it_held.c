void *malloc(int n);
void free(void *p);
void *realloc(void *p, int n);
void drop_inner(int **box);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    *tab = p;
    int **bigger = realloc(tab, 16);
    if (bigger == 0) {
        return 0;
    }
    drop_inner(bigger);
    return *p;
}
