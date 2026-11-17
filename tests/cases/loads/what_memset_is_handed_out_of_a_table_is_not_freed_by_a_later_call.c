void *malloc(int n);
void *memset(void *p, int c, int n);
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
    memset(*tab, 0, 4);
    log_line();
    return *p;
}
