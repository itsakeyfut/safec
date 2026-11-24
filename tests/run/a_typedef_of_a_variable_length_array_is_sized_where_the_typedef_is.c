static int calls;

static int counted(void) {
    calls++;
    return 4;
}

int main(void) {
    int wrong = 0;

    // The length is worked out where the typedef is (C17 6.7.8 p3), not
    // where an object of the type is declared.
    int n = 3;
    typedef char Later[n];
    n = 7;
    Later later;
    wrong += sizeof later == 3 ? 0 : 1;

    // Once, however many objects are declared with it.
    int m = 3;
    typedef char Once[m++];
    Once first;
    Once second;
    wrong += m == 4 && sizeof first == 3 && sizeof second == 3 ? 0 : 1;

    // Even with no object at all.
    typedef int Unused[counted()];
    wrong += calls == 1 ? 0 : 1;

    // `sizeof` of the type itself, when it runs.
    int k = 5;
    typedef long long Wide[k];
    wrong += sizeof(Wide) == 5 * sizeof(long long) ? 0 : 1;

    // Afresh on each pass of a loop.
    int total = 0;
    for (int i = 1; i <= 3; i++) {
        typedef char Pass[i];
        Pass a;
        total += (int)sizeof a;
    }
    wrong += total == 6 ? 0 : 1;

    // The `n` in scope where the typedef is, not a later one that hides it.
    int s = 3;
    typedef char Scoped[s];
    {
        int s = 9;
        Scoped a;
        wrong += sizeof a == 3 && s == 9 ? 0 : 1;
    }
    return wrong;
}
