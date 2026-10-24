void *malloc(int n);
void stash(char c);
void release_stashed(void);

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
    void *vs = tab;
    char *s = vs;
    int i = 0;
    while (i < 8) {
        if (s != 0) {
            stash(*s);
        }
        s = s + 1;
        i = i + 1;
    }
    release_stashed();
    return *p;
}
