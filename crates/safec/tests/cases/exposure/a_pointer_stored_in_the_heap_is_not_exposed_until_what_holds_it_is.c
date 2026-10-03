void *malloc(int n);
void free(void *p);
void log_line(void);

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
    log_line();
    int r = *p;
    free(p);
    free(tab);
    return r;
}
