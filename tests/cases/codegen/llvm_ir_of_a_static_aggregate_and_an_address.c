struct Entry {
    const char *name;
    char tag;
    long long size;
};

int g;
struct Entry table[3] = {{"a", 1, 2}, {"bc", 3}};
int *at = &g;
