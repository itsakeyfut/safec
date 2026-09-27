void *malloc(int n);
void keep(int **q);

int main(void) {
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    *tab = p;
    int *q = *tab;
    keep(&q);
    return *p;
}
