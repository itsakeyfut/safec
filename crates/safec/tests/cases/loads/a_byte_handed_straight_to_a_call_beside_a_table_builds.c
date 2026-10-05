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
    char **lines = malloc(8);
    if (lines == 0) {
        return 0;
    }
    char *line = malloc(4);
    if (line == 0) {
        return 0;
    }
    *line = 65;
    *lines = line;
    char *first = *lines;
    if (first != 0) {
        emit(*first);
    }
    emit(*line);
    int r = **tab;
    free(line);
    free(lines);
    free(p);
    free(tab);
    return r;
}
