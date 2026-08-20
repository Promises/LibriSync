import React, { useState, useEffect } from 'react';
import { Alert, Linking } from 'react-native';
import { NavigationContainer } from '@react-navigation/native';
import { createBottomTabNavigator } from '@react-navigation/bottom-tabs';
import { createNativeStackNavigator } from '@react-navigation/native-stack';
import { Ionicons } from '@expo/vector-icons';
import Constants from 'expo-constants';
import * as SecureStore from 'expo-secure-store';
import { useTheme } from '../styles/theme';
import { checkForUpdate, isGithubReleaseBuild } from '../utils/versionCheck';

import LibraryScreen from '../screens/LibraryScreen';
import AccountsScreen from '../screens/AccountsScreen';
import LibriVoxBrowseScreen from '../screens/LibriVoxBrowseScreen';
import SettingsScreen from '../screens/SettingsScreen';
import TaskDebugScreen from '../screens/TaskDebugScreen';

const Tab = createBottomTabNavigator();
const AccountsStack = createNativeStackNavigator();
const DEBUG_MODE_KEY = 'debug_mode_enabled';

/**
 * One unified Accounts screen for every provider, with LibriVox — which needs no
 * account — pushed on top as a browse destination from the header action.
 *
 * Both screens draw their own headers, hence `headerShown: false` throughout.
 */
function AccountsStackScreen() {
  return (
    <AccountsStack.Navigator screenOptions={{ headerShown: false }}>
      <AccountsStack.Screen name="AccountsList" component={AccountsScreen} />
      <AccountsStack.Screen name="LibriVoxBrowse" component={LibriVoxBrowseScreen} />
    </AccountsStack.Navigator>
  );
}

export default function AppNavigator() {
  const { colors } = useTheme();
  const [enableDebugScreen, setEnableDebugScreen] = useState<boolean>(
    Constants.expoConfig?.extra?.enableDebugScreen ?? __DEV__
  );

  // Check if debug mode is enabled/disabled via SecureStore (secret activation/deactivation)
  useEffect(() => {
    const checkDebugMode = async () => {
      try {
        const debugEnabled = await SecureStore.getItemAsync(DEBUG_MODE_KEY);
        if (debugEnabled === 'true') {
          setEnableDebugScreen(true);
        } else if (debugEnabled === 'false') {
          setEnableDebugScreen(false);
        }
      } catch (error) {
        console.error('[AppNavigator] Failed to check debug mode:', error);
      }
    };
    checkDebugMode();
  }, []);

  // Check for app updates on GitHub release builds
  useEffect(() => {
    if (!isGithubReleaseBuild()) return;

    const checkUpdate = async () => {
      const update = await checkForUpdate();
      if (update?.isUpdateAvailable) {
        Alert.alert(
          'Update Available',
          `A new version of LibriSync is available.\n\nCurrent: v${update.currentVersion}\nLatest: v${update.latestVersion}`,
          [
            { text: 'Later', style: 'cancel' },
            {
              text: 'Download',
              onPress: () => Linking.openURL(update.downloadUrl),
            },
          ],
        );
      }
    };
    checkUpdate();
  }, []);

  return (
    <NavigationContainer>
      <Tab.Navigator
        screenOptions={{
          tabBarStyle: {
            backgroundColor: colors.backgroundSecondary,
            borderTopColor: colors.border,
            borderTopWidth: 1,
          },
          tabBarActiveTintColor: colors.accent,
          tabBarInactiveTintColor: colors.textSecondary,
          headerStyle: {
            backgroundColor: colors.backgroundSecondary,
            borderBottomColor: colors.border,
            borderBottomWidth: 1,
          },
          headerTintColor: colors.textPrimary,
          headerTitleStyle: {
            fontWeight: '600',
          },
        }}
      >
        <Tab.Screen
          name="Library"
          component={LibraryScreen}
          options={{
            tabBarLabel: 'Library',
            headerShown: false,
            tabBarIcon: ({ color, size }) => (
              <Ionicons name="library" size={size} color={color} />
            ),
          }}
        />
        <Tab.Screen
          name="Accounts"
          component={AccountsStackScreen}
          options={{
            tabBarLabel: 'Accounts',
            headerShown: false,
            tabBarIcon: ({ color, size }) => (
              <Ionicons name="person" size={size} color={color} />
            ),
          }}
        />
        <Tab.Screen
          name="Settings"
          component={SettingsScreen}
          options={{
            tabBarLabel: 'Settings',
            headerShown: false,
            tabBarIcon: ({ color, size }) => (
              <Ionicons name="settings" size={size} color={color} />
            ),
          }}
        />
        {enableDebugScreen && (
          <Tab.Screen
            name="Debug"
            component={TaskDebugScreen}
            options={{
              tabBarLabel: 'Debug',
              headerShown: false,
              tabBarIcon: ({ color, size }) => (
                <Ionicons name="bug" size={size} color={color} />
              ),
            }}
          />
        )}
      </Tab.Navigator>
    </NavigationContainer>
  );
}
