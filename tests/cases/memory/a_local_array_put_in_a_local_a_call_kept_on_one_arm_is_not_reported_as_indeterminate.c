int choose(void);
void stash(int ***pp);
void tick(void);

int main(void) {
    int *m[1];
    int **p = 0;
    if (choose()) {
        stash(&p);
    }
    p = m;
    tick();
    int *q = m[0];
    return q == 0;
}
