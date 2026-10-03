void *malloc(int n);
void release(int **t);

int main(void) {
    int ***holder = malloc(8);
    if (holder == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int i = 0;
    while (1) {
        int **tab = malloc(8);
        if (tab == 0) {
            return 0;
        }
        if (i == 1) {
            release(*holder);
            return *p;
        }
        *tab = p;
        *holder = tab;
        i = i + 1;
    }
    return 0;
}
