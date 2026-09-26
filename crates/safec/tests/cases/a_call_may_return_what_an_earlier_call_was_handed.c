void *malloc(int n);
void free(void *p);
void stash(int *p);
int *fetch(void);
void drop_stashed(void);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    stash(p);
    int *q = fetch();
    drop_stashed();
    free(q);
    return 0;
}
