void *malloc(int n);
void *memset(void *d, int c, int n);
int strcmp(char *a, char *b);
int strncmp(char *a, char *b, int n);
char *strstr(char *a, char *b);
char *strchr(char *s, int c);
char *strrchr(char *s, int c);

int ask_strcmp(void) {
    char *p = malloc(8);
    if (p == 0) {
        return 1;
    }
    memset(p, 0, 8);
    return strcmp(p, p + 1) == 0;
}

int ask_strncmp(void) {
    char *p = malloc(8);
    if (p == 0) {
        return 1;
    }
    memset(p, 0, 8);
    return strncmp(p, p + 2, 4) == 0;
}

int ask_strstr(void) {
    char *p = malloc(8);
    if (p == 0) {
        return 1;
    }
    memset(p, 0, 8);
    return strstr(p, p + 3) == 0;
}

int ask_strchr(void) {
    char *p = malloc(8);
    if (p == 0) {
        return 1;
    }
    memset(p, 0, 8);
    return strchr(p + 4, 0) == 0;
}

int ask_strrchr(void) {
    char *p = malloc(8);
    if (p == 0) {
        return 1;
    }
    memset(p, 0, 8);
    return strrchr(p + 5, 0) == 0;
}

int main(void) {
    return ask_strcmp() + ask_strncmp() + ask_strstr() + ask_strchr() + ask_strrchr();
}
